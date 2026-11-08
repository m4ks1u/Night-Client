//! Files in an instance's content folders: `mods`, `resourcepacks` and
//! `shaderpacks` (see `ContentKind`).
//!
//! A disabled file is the same file renamed to `<name>.jar.disabled` (or
//! `.zip.disabled`), which loaders, the game and shader mods all ignore. Downloads go through the verified engine (SHA-1 and
//! size), then SHA-512 is checked as well before the file is kept.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha512};
use tokio_util::sync::CancellationToken;

use nc_core::content::ContentKind;

use super::api::{Version, trusted_file_url, valid_file_name, valid_mod_file_name};
use crate::{
    LauncherError,
    download::{DownloadTask, Downloader},
    fsutil,
};

pub const DISABLED_SUFFIX: &str = ".disabled";
/// Mods larger than this are refused on import.
pub const MAX_LOCAL_JAR: u64 = 256 * 1024 * 1024;

pub fn mods_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("mods")
}

/// Folder for one kind of content inside a game directory.
pub fn content_dir(game_dir: &Path, kind: ContentKind) -> PathBuf {
    game_dir.join(kind.folder())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    /// Enabled file name (`x.jar`), also for disabled files.
    pub file_name: String,
    pub enabled: bool,
    pub size: u64,
    pub sha1: String,
    pub sha512: String,
}

pub(crate) fn sha512_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha512::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn path_for(
    dir: &Path,
    kind: ContentKind,
    file_name: &str,
    enabled: bool,
) -> Result<PathBuf, LauncherError> {
    if !valid_file_name(kind, file_name) {
        return Err(LauncherError::InvalidPath(format!(
            "{file_name}: not a valid {} file name",
            kind.as_str()
        )));
    }
    Ok(if enabled {
        dir.join(file_name)
    } else {
        dir.join(format!("{file_name}{DISABLED_SUFFIX}"))
    })
}

/// Current files of `kind` in `dir` (hashing each; call off the async runtime).
pub fn scan(dir: &Path, kind: ContentKind) -> Result<Vec<ModFile>, LauncherError> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let (file_name, enabled) = match name.strip_suffix(DISABLED_SUFFIX) {
            Some(n) => (n.to_owned(), false),
            None => (name.clone(), true),
        };
        if !valid_file_name(kind, &file_name) {
            continue; // not this kind of file (configs, .txt, …)
        }
        let path = e.path();
        out.push(ModFile {
            size: e.metadata().map(|m| m.len()).unwrap_or(0),
            sha1: fsutil::sha1_file(&path).map_err(LauncherError::io(&path))?,
            sha512: sha512_file(&path).map_err(LauncherError::io(&path))?,
            file_name,
            enabled,
        });
    }
    out.sort_by_key(|m| m.file_name.to_lowercase());
    Ok(out)
}

/// Download a version's primary file into `dir`, verified by SHA-1, size
/// and SHA-512. An existing file with the same name is replaced atomically.
pub async fn install_version(
    dl: &Downloader,
    dir: &Path,
    kind: ContentKind,
    version: &Version,
    cancel: &CancellationToken,
) -> Result<ModFile, LauncherError> {
    let file = version.primary_file().ok_or_else(|| LauncherError::Parse {
        what: version.name.clone(),
        message: "version has no files".into(),
    })?;
    if !trusted_file_url(&file.url) {
        return Err(LauncherError::InvalidPath(format!(
            "{}: files must come from cdn.modrinth.com",
            file.url
        )));
    }
    let dest = path_for(dir, kind, &file.filename, true)?;
    dl.fetch_all(
        vec![DownloadTask {
            url: file.url.clone(),
            dest: dest.clone(),
            sha1: Some(file.hashes.sha1.clone()),
            size: Some(file.size),
        }],
        cancel,
        |_| {},
    )
    .await?;
    let d = dest.clone();
    let actual = tokio::task::spawn_blocking(move || sha512_file(&d))
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))?
        .map_err(LauncherError::io(&dest))?;
    if !actual.eq_ignore_ascii_case(&file.hashes.sha512) {
        let _ = std::fs::remove_file(&dest);
        return Err(LauncherError::Download(
            crate::download::DownloadError::Integrity {
                url: file.url.clone(),
                expected: file.hashes.sha512.clone(),
                actual,
            },
        ));
    }
    // A disabled copy of the same file would load nothing but confuse the list.
    let _ = std::fs::remove_file(path_for(dir, kind, &file.filename, false)?);
    Ok(ModFile {
        file_name: file.filename.clone(),
        enabled: true,
        size: file.size,
        sha1: file.hashes.sha1.to_ascii_lowercase(),
        sha512: actual,
    })
}

pub fn set_enabled(
    dir: &Path,
    kind: ContentKind,
    file_name: &str,
    enabled: bool,
) -> Result<(), LauncherError> {
    let from = path_for(dir, kind, file_name, !enabled)?;
    let to = path_for(dir, kind, file_name, enabled)?;
    if to.exists() && !from.exists() {
        return Ok(()); // already in the requested state
    }
    std::fs::rename(&from, &to).map_err(LauncherError::io(&from))
}

pub fn remove(dir: &Path, kind: ContentKind, file_name: &str) -> Result<(), LauncherError> {
    for enabled in [true, false] {
        let p = path_for(dir, kind, file_name, enabled)?;
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(LauncherError::Io { path: p, source: e }),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JarInfo {
    /// Loader the jar declares itself for, from its metadata files.
    pub loader: Option<String>,
    pub mod_id: Option<String>,
}

/// Read a jar's loader metadata without extracting or running anything.
pub fn inspect_jar(path: &Path) -> Result<JarInfo, LauncherError> {
    let f = std::fs::File::open(path).map_err(LauncherError::io(path))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| {
        LauncherError::InvalidPath(format!("{}: not a valid jar (zip) file", path.display()))
    })?;
    let mut read = |name: &str| -> Option<String> {
        let mut e = z.by_name(name).ok()?;
        let mut s = String::new();
        (&mut e).take(256 * 1024).read_to_string(&mut s).ok()?;
        Some(s)
    };
    if let Some(json) = read("fabric.mod.json") {
        let id = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v.get("id")?.as_str().map(str::to_owned))
            .filter(|id| plausible_mod_id(id));
        return Ok(JarInfo {
            loader: Some("fabric".into()),
            mod_id: id,
        });
    }
    if read("quilt.mod.json").is_some() {
        return Ok(JarInfo {
            loader: Some("quilt".into()),
            mod_id: None,
        });
    }
    if let Some(toml) = read("META-INF/neoforge.mods.toml") {
        return Ok(JarInfo {
            loader: Some("neoforge".into()),
            mod_id: toml_mod_id(&toml),
        });
    }
    if let Some(toml) = read("META-INF/mods.toml") {
        return Ok(JarInfo {
            loader: Some("forge".into()),
            mod_id: toml_mod_id(&toml),
        });
    }
    Ok(JarInfo {
        loader: None,
        mod_id: None,
    })
}

fn toml_mod_id(toml: &str) -> Option<String> {
    toml.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("modId"))
        .and_then(|rest| rest.split('"').nth(1))
        .filter(|id| plausible_mod_id(id))
        .map(str::to_owned)
}

/// Mod ids come from an untrusted file and are stored and displayed, so
/// only keep ones in the shape Fabric and NeoForge require: 2–64 ASCII
/// letters, digits, `_`, `-` or `.`.
fn plausible_mod_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Copy a user-chosen jar into the mods folder after validation:
/// `.jar`, ≤ 256 MiB, a readable zip, and not declared for a different
/// loader (`instance_loader` is `fabric`, `quilt` or `neoforge`; Quilt
/// also accepts Fabric jars).
pub fn import_local(
    dir: &Path,
    src: &Path,
    instance_loader: &str,
) -> Result<(ModFile, JarInfo), LauncherError> {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !valid_mod_file_name(&name) {
        return Err(LauncherError::InvalidPath(format!(
            "{name}: choose a .jar file"
        )));
    }
    let meta = std::fs::metadata(src).map_err(LauncherError::io(src))?;
    if !meta.is_file() || meta.len() > MAX_LOCAL_JAR {
        return Err(LauncherError::InvalidPath(format!(
            "{name}: not a file, or larger than 256 MB"
        )));
    }
    let info = inspect_jar(src)?;
    if let Some(l) = &info.loader
        && !nc_core::loader::mod_loaders(instance_loader).contains(&l.as_str())
    {
        return Err(LauncherError::InvalidPath(format!(
            "{name} is a {l} mod; this instance uses {instance_loader}"
        )));
    }
    std::fs::create_dir_all(dir).map_err(LauncherError::io(dir))?;
    let dest = path_for(dir, ContentKind::Mod, &name, true)?;
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    std::fs::copy(src, &tmp).map_err(LauncherError::io(&tmp))?;
    std::fs::rename(&tmp, &dest).map_err(LauncherError::io(&dest))?;
    Ok((
        ModFile {
            sha1: fsutil::sha1_file(&dest).map_err(LauncherError::io(&dest))?,
            sha512: sha512_file(&dest).map_err(LauncherError::io(&dest))?,
            size: meta.len(),
            file_name: name,
            enabled: true,
        },
        info,
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn jar(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let p = dir.join(name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, body) in entries {
            w.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        p
    }

    #[test]
    fn scan_toggle_remove() {
        let d = tempfile::tempdir().unwrap();
        let mods = d.path().join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        jar(&mods, "a.jar", &[("fabric.mod.json", r#"{"id":"a"}"#)]);
        jar(&mods, "b.jar", &[]);
        std::fs::write(mods.join("notes.txt"), "x").unwrap();

        assert_eq!(scan(&mods, ContentKind::Mod).unwrap().len(), 2);
        set_enabled(&mods, ContentKind::Mod, "b.jar", false).unwrap();
        assert!(mods.join("b.jar.disabled").is_file());
        let list = scan(&mods, ContentKind::Mod).unwrap();
        assert_eq!(
            (list[1].file_name.as_str(), list[1].enabled),
            ("b.jar", false)
        );
        assert_eq!(list[0].sha512.len(), 128);
        set_enabled(&mods, ContentKind::Mod, "b.jar", false).unwrap(); // idempotent
        set_enabled(&mods, ContentKind::Mod, "b.jar", true).unwrap();
        remove(&mods, ContentKind::Mod, "a.jar").unwrap();
        remove(&mods, ContentKind::Mod, "a.jar").unwrap(); // idempotent
        assert_eq!(scan(&mods, ContentKind::Mod).unwrap().len(), 1);
        assert!(set_enabled(&mods, ContentKind::Mod, "../evil.jar", true).is_err());
        assert!(remove(&mods, ContentKind::Mod, "..\\x.jar").is_err());
    }

    #[test]
    fn pack_folders_list_only_zip_files() {
        let d = tempfile::tempdir().unwrap();
        let packs = content_dir(d.path(), ContentKind::ResourcePack);
        assert!(packs.ends_with("resourcepacks"));
        std::fs::create_dir_all(packs.join("unpacked")).unwrap();
        std::fs::write(packs.join("faithful.zip"), b"PK").unwrap();
        std::fs::write(packs.join("old.zip.disabled"), b"PK").unwrap();
        std::fs::write(packs.join("stray.jar"), b"PK").unwrap();
        std::fs::write(packs.join(".zip"), b"PK").unwrap();
        let list = scan(&packs, ContentKind::ResourcePack).unwrap();
        assert_eq!(
            list.iter()
                .map(|f| (f.file_name.as_str(), f.enabled))
                .collect::<Vec<_>>(),
            [("faithful.zip", true), ("old.zip", false)],
            "folders, other extensions and a bare extension are skipped"
        );
        set_enabled(&packs, ContentKind::ResourcePack, "faithful.zip", false).unwrap();
        assert!(packs.join("faithful.zip.disabled").is_file());
        assert!(set_enabled(&packs, ContentKind::ResourcePack, "stray.jar", false).is_err());
        assert!(
            scan(&packs, ContentKind::Mod).unwrap().len() == 1,
            "the .jar, as a mod"
        );
        assert!(
            scan(&d.path().join("shaderpacks"), ContentKind::Shader)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn inspects_loader_metadata() {
        let d = tempfile::tempdir().unwrap();
        let f = jar(
            d.path(),
            "f.jar",
            &[("fabric.mod.json", r#"{"id":"sodium","name":"Sodium"}"#)],
        );
        assert_eq!(
            inspect_jar(&f).unwrap(),
            JarInfo {
                loader: Some("fabric".into()),
                mod_id: Some("sodium".into())
            }
        );
        let n = jar(
            d.path(),
            "n.jar",
            &[("META-INF/neoforge.mods.toml", "[[mods]]\nmodId=\"jei\"\n")],
        );
        assert_eq!(inspect_jar(&n).unwrap().loader.as_deref(), Some("neoforge"));
        assert_eq!(inspect_jar(&n).unwrap().mod_id.as_deref(), Some("jei"));
        std::fs::write(d.path().join("bad.jar"), "not a zip").unwrap();
        assert!(inspect_jar(&d.path().join("bad.jar")).is_err());

        // Hostile metadata: the loader is still detected, the id is dropped.
        let long = format!(r#"{{"id":"{}"}}"#, "a".repeat(5000));
        let h = jar(d.path(), "h.jar", &[("fabric.mod.json", long.as_str())]);
        assert_eq!(
            inspect_jar(&h).unwrap(),
            JarInfo {
                loader: Some("fabric".into()),
                mod_id: None
            }
        );
        let t = jar(
            d.path(),
            "t.jar",
            &[(
                "META-INF/mods.toml",
                "modId=\"<img src=x onerror=alert(1)>\"\n",
            )],
        );
        assert_eq!(inspect_jar(&t).unwrap().mod_id, None);
    }

    #[test]
    fn local_import_validates() {
        let d = tempfile::tempdir().unwrap();
        let mods = d.path().join("mods");
        let fabric = jar(
            d.path(),
            "lithium.jar",
            &[("fabric.mod.json", r#"{"id":"lithium"}"#)],
        );
        let (m, info) = import_local(&mods, &fabric, "fabric").unwrap();
        assert!(m.enabled && mods.join("lithium.jar").is_file());
        assert_eq!(info.mod_id.as_deref(), Some("lithium"));
        assert!(
            import_local(&mods, &fabric, "neoforge").is_err(),
            "wrong loader refused"
        );
        let qmods = d.path().join("qmods");
        assert!(
            import_local(&qmods, &fabric, "quilt").is_ok(),
            "Quilt runs Fabric mods"
        );
        let quilt = jar(d.path(), "q.jar", &[("quilt.mod.json", "{}")]);
        assert!(
            import_local(&mods, &quilt, "fabric").is_err(),
            "Fabric does not run Quilt mods"
        );
        let unknown = jar(d.path(), "plain.jar", &[]);
        assert!(
            import_local(&mods, &unknown, "neoforge").is_ok(),
            "unknown loader allowed (with a warning in the UI)"
        );
        std::fs::write(d.path().join("x.exe"), "MZ").unwrap();
        assert!(import_local(&mods, &d.path().join("x.exe"), "fabric").is_err());
    }
}
