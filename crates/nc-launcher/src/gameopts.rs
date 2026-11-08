//! Turning resource packs and Iris shader packs on and off in an instance's
//! game settings (`options.txt`, `config/iris.properties`). The text edits
//! live in `nc_core::gameopts`; this module reads and writes the files
//! (size-capped, written to a temp file and renamed).

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use nc_core::gameopts::{
    iris_shader, pack_id, resource_packs, set_iris_shader, set_resource_packs, toggle_pack,
    uses_file_prefix,
};

use crate::LauncherError;

/// Settings files larger than this are not ours to rewrite.
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

pub fn options_path(game_dir: &Path) -> PathBuf {
    game_dir.join("options.txt")
}

pub fn iris_path(game_dir: &Path) -> PathBuf {
    game_dir.join("config").join("iris.properties")
}

/// The file's text, `None` when it doesn't exist. Iris writes
/// ISO-8859-1, so bytes that aren't UTF-8 are read as Latin-1.
fn read(path: &Path) -> Result<Option<String>, LauncherError> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LauncherError::io(path)(e)),
    };
    let mut buf = Vec::new();
    f.take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(LauncherError::io(path))?;
    if buf.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(LauncherError::Parse {
            what: path.display().to_string(),
            message: "the settings file is unexpectedly large".into(),
        });
    }
    Ok(Some(match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| char::from(b)).collect(),
    }))
}

fn write(path: &Path, text: &str) -> Result<(), LauncherError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(LauncherError::io(p))?;
    }
    let tmp = path.with_extension("nc-tmp");
    std::fs::write(&tmp, text).map_err(LauncherError::io(&tmp))?;
    std::fs::rename(&tmp, path).map_err(LauncherError::io(path))
}

/// The data version of a client jar (`version.json` → `world_version`,
/// Minecraft 1.14+), used as `version:` when options.txt is created, so the
/// game doesn't treat the new file as coming from an old version.
pub fn jar_data_version(jar: &Path) -> Option<u32> {
    let f = std::fs::File::open(jar).ok()?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).ok()?;
    let mut e = z.by_name("version.json").ok()?;
    let mut s = String::new();
    (&mut e).take(64 * 1024).read_to_string(&mut s).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("world_version")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
}

/// File names of the resource packs turned on in the game, lowest priority
/// first. Built-in packs (vanilla, mod packs) are left out.
pub fn active_resource_packs(
    game_dir: &Path,
    minecraft: &str,
) -> Result<Vec<String>, LauncherError> {
    let prefix = uses_file_prefix(minecraft);
    let text = read(&options_path(game_dir))?.unwrap_or_default();
    Ok(resource_packs(&text)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| {
            if prefix {
                id.strip_prefix("file/").map(str::to_owned)
            } else {
                Some(id)
            }
        })
        .filter(|n| !n.contains('/') && n != "vanilla")
        .collect())
}

/// Turn a resource pack file on (on top) or off. `data_version` is written
/// when options.txt has to be created.
pub fn set_resource_pack(
    game_dir: &Path,
    minecraft: &str,
    file_name: &str,
    on: bool,
    data_version: Option<u32>,
) -> Result<(), LauncherError> {
    let prefix = uses_file_prefix(minecraft);
    let path = options_path(game_dir);
    let existing = read(&path)?;
    if existing.is_none() && !on {
        return Ok(());
    }
    let text = existing.unwrap_or_else(|| {
        data_version
            .map(|v| format!("version:{v}\n"))
            .unwrap_or_default()
    });
    let next = toggle_pack(
        resource_packs(&text),
        &pack_id(file_name, prefix),
        on,
        prefix,
    );
    write(&path, &set_resource_packs(&text, &next))
}

/// After a pack file is renamed (updated) or removed (`new` = `None`),
/// point the game's settings at the new file, keeping its place, or drop it.
/// Does nothing when the old file wasn't turned on.
pub fn follow_pack(
    game_dir: &Path,
    minecraft: &str,
    shader: bool,
    old: &str,
    new: Option<&str>,
) -> Result<(), LauncherError> {
    if shader {
        if active_shader(game_dir)?.as_deref() == Some(old) {
            match new {
                Some(n) => set_shader(game_dir, Some(n))?,
                None => set_shader(game_dir, None)?,
            }
        }
        return Ok(());
    }
    let prefix = uses_file_prefix(minecraft);
    let path = options_path(game_dir);
    let Some(text) = read(&path)? else {
        return Ok(());
    };
    let Some(list) = resource_packs(&text) else {
        return Ok(());
    };
    let old_id = pack_id(old, prefix);
    if !list.contains(&old_id) {
        return Ok(());
    }
    let next: Vec<String> = list
        .into_iter()
        .filter_map(|id| {
            if id == old_id {
                new.map(|n| pack_id(n, prefix))
            } else {
                Some(id)
            }
        })
        .collect();
    write(&path, &set_resource_packs(&text, &next))
}

/// The shader pack Iris is set to use, when shaders are on.
pub fn active_shader(game_dir: &Path) -> Result<Option<String>, LauncherError> {
    Ok(read(&iris_path(game_dir))?.and_then(|t| iris_shader(&t)))
}

/// Set Iris's shader pack (`Some`) or turn shaders off (`None`).
pub fn set_shader(game_dir: &Path, pack: Option<&str>) -> Result<(), LauncherError> {
    let path = iris_path(game_dir);
    let existing = read(&path)?;
    if existing.is_none() && pack.is_none() {
        return Ok(());
    }
    write(&path, &set_iris_shader(&existing.unwrap_or_default(), pack))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn resource_packs_on_and_off_on_disk() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        // Turning off with no options.txt creates nothing.
        set_resource_pack(g, "1.21.1", "a.zip", false, Some(3955)).unwrap();
        assert!(!options_path(g).exists());
        set_resource_pack(g, "1.21.1", "a.zip", true, Some(3955)).unwrap();
        assert_eq!(
            std::fs::read_to_string(options_path(g)).unwrap(),
            "version:3955\nresourcePacks:[\"vanilla\",\"file/a.zip\"]\n"
        );
        set_resource_pack(g, "1.21.1", "b.zip", true, None).unwrap();
        assert_eq!(
            active_resource_packs(g, "1.21.1").unwrap(),
            ["a.zip", "b.zip"]
        );
        set_resource_pack(g, "1.21.1", "a.zip", false, None).unwrap();
        assert_eq!(active_resource_packs(g, "1.21.1").unwrap(), ["b.zip"]);
        assert!(!g.join("options.nc-tmp").exists());
        // An update renames the file in place; a removal drops it.
        set_resource_pack(g, "1.21.1", "c.zip", true, None).unwrap();
        follow_pack(g, "1.21.1", false, "b.zip", Some("b2.zip")).unwrap();
        assert_eq!(
            active_resource_packs(g, "1.21.1").unwrap(),
            ["b2.zip", "c.zip"]
        );
        follow_pack(g, "1.21.1", false, "c.zip", None).unwrap();
        follow_pack(g, "1.21.1", false, "not-on.zip", None).unwrap();
        assert_eq!(active_resource_packs(g, "1.21.1").unwrap(), ["b2.zip"]);
        // Old versions: no prefix.
        let old = tempfile::tempdir().unwrap();
        set_resource_pack(old.path(), "1.12.2", "c.zip", true, None).unwrap();
        assert_eq!(
            active_resource_packs(old.path(), "1.12.2").unwrap(),
            ["c.zip"]
        );
    }

    #[test]
    fn iris_on_disk_and_latin1() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        assert_eq!(active_shader(g).unwrap(), None);
        set_shader(g, None).unwrap();
        assert!(!iris_path(g).exists());
        // A file Iris wrote, with a Latin-1 comment byte.
        std::fs::create_dir_all(g.join("config")).unwrap();
        std::fs::write(iris_path(g), b"#Caf\xe9\nenableShaders=false\n").unwrap();
        set_shader(g, Some("BSL.zip")).unwrap();
        assert_eq!(active_shader(g).unwrap().as_deref(), Some("BSL.zip"));
        follow_pack(g, "1.21.1", true, "BSL.zip", Some("BSL2.zip")).unwrap();
        assert_eq!(active_shader(g).unwrap().as_deref(), Some("BSL2.zip"));
        follow_pack(g, "1.21.1", true, "other.zip", None).unwrap();
        assert_eq!(active_shader(g).unwrap().as_deref(), Some("BSL2.zip"));
        set_shader(g, None).unwrap();
        assert_eq!(active_shader(g).unwrap(), None);
    }

    #[test]
    fn data_version_from_client_jar() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut w = zip::ZipWriter::new(f.reopen().unwrap());
        w.start_file("version.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(br#"{"id":"1.21.1","world_version":3955}"#)
            .unwrap();
        w.finish().unwrap();
        assert_eq!(jar_data_version(f.path()), Some(3955));
        assert_eq!(jar_data_version(Path::new("missing.jar")), None);
    }
}
