//! NeoForge and Forge (1.12.2+): runs the installer's processors natively
//! (no installer GUI). Both ship the same `install_profile.json` format
//! (spec 1; Forge 1.12.2 to 1.16.5 use spec 0, which is the same shape).
//!
//! 1. Install vanilla, then download the official installer jar, verified
//!    against the Maven `.sha1` sidecar.
//! 2. Read `install_profile.json` (spec 0 or 1) and `version.json` from it.
//! 3. Download every installer and version library through the verified
//!    download engine.
//! 4. Extract `/data/...` entries with zip-slip-safe extraction, and
//!    libraries listed without a URL from the installer's `maven/` folder,
//!    verified against their declared SHA-1.
//! 5. Provide Mojang mappings ourselves (verified) and skip the processor
//!    that would download them, so processors make no unverified downloads.
//! 6. Run each client processor as `java -cp <jar;classpath> <Main-Class>
//!    args` with the managed runtime: no shell, no console window,
//!    cancellable, with a timeout. Declared output hashes are verified.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use nc_core::{
    installprofile::{InstallProfile, ResolveContext, maven_path, resolve_arg},
    libraries,
    loader::{InstallKey, forge_versions_for, neoforge_versions_for},
    manifest::VersionJson,
    rules::Environment,
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::LoaderVersion;
use crate::{
    LauncherError,
    download::{DownloadTask, Progress, Summary},
    fsutil,
    install::{self, InstallProgress, Installed, Phase, ProgressFn},
    meta::Meta,
};

const PROCESSOR_TIMEOUT: Duration = Duration::from_secs(600);

/// Mojang runtime (Java 17) used for spec 0 processors when the game itself
/// runs on Java 8. Forge computed the declared output hashes with a newer
/// Java: Java 8u51 deflates differently, so jarsplitter's outputs (Forge
/// 1.13.2 to 1.16.5) would never match.
const SPEC0_PROCESSOR_RUNTIME: &str = "java-runtime-gamma";

/// NeoForge and (modern) Forge ship the same installer format; they differ
/// only in where the installer lives and how versions are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dist {
    NeoForge,
    /// Forge for Minecraft 1.12.2 (build 2851+) and newer.
    Forge,
}

impl Dist {
    fn name(self) -> &'static str {
        match self {
            Self::NeoForge => "NeoForge",
            Self::Forge => "Forge",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::NeoForge => "neoforge",
            Self::Forge => "forge",
        }
    }

    /// `<maven>/<group path>/<artifact>`
    fn artifact_url(self, meta: &Meta) -> String {
        match self {
            Self::NeoForge => format!("{}/net/neoforged/neoforge", meta.neoforge_maven_url),
            Self::Forge => format!("{}/net/minecraftforge/forge", meta.forge_maven_url),
        }
    }

    /// The Maven version of the installer for a key's loader version.
    fn maven_version(self, minecraft: &str, loader_version: &str) -> String {
        match self {
            Self::NeoForge => loader_version.to_owned(),
            Self::Forge => format!("{minecraft}-{loader_version}"),
        }
    }
}

pub async fn versions(
    dist: Dist,
    meta: &Meta,
    minecraft: &str,
    cancel: &CancellationToken,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    let url = format!("{}/maven-metadata.xml", dist.artifact_url(meta));
    let cache = meta
        .layout
        .cache_dir()
        .join(format!("{}-maven-metadata.xml", dist.slug()));
    let xml = match meta.dl.get_bytes(&url, cancel).await {
        Ok(b) => {
            let c = cache.clone();
            let b2 = b.clone();
            let _ = tokio::task::spawn_blocking(move || fsutil::atomic_write(&c, &b2)).await;
            b
        }
        Err(crate::download::DownloadError::Cancelled) => return Err(LauncherError::Cancelled),
        Err(e) => tokio::fs::read(&cache)
            .await
            .map_err(|_| LauncherError::Download(e))?,
    };
    let xml = String::from_utf8_lossy(&xml);
    let all: Vec<&str> = xml
        .split("<version>")
        .skip(1)
        .filter_map(|s| s.split_once("</version>").map(|(v, _)| v.trim()))
        .collect();
    Ok(match dist {
        Dist::NeoForge => neoforge_versions_for(minecraft, all)
            .into_iter()
            .map(|(version, stable)| LoaderVersion { version, stable })
            .collect(),
        Dist::Forge => forge_versions_for(minecraft, all)
            .into_iter()
            .map(|version| LoaderVersion {
                version,
                stable: true,
            })
            .collect(),
    })
}

fn read_zip_entry(jar: &Path, name: &str) -> Result<Vec<u8>, LauncherError> {
    let f = std::fs::File::open(jar).map_err(LauncherError::io(jar))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| LauncherError::Extract(e.into()))?;
    let mut entry = z
        .by_name(name)
        .map_err(|e| LauncherError::Extract(e.into()))?;
    let mut buf = Vec::new();
    (&mut entry)
        .take(64 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(LauncherError::io(jar))?;
    Ok(buf)
}

/// `Main-Class` from a jar manifest (handles 72-byte line continuations).
pub fn main_class(manifest: &str) -> Option<String> {
    let mut joined = String::new();
    for line in manifest.lines() {
        if let Some(cont) = line.strip_prefix(' ') {
            joined.push_str(cont);
        } else {
            joined.push('\n');
            joined.push_str(line);
        }
    }
    joined
        .lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Upper bound for everything extracted from one installer jar. Real
/// installers hold a few MB of data entries; this only stops a zip bomb.
const MAX_INSTALLER_EXTRACT: u64 = 512 * 1024 * 1024;

/// A library the profile lists without a download URL. Older Forge installers
/// (spec 0) ship these inside the jar under `maven/<path>`.
#[derive(Debug, Clone)]
struct EmbeddedLib {
    path: String,
    dest: PathBuf,
    sha1: Option<String>,
}

/// Extract URL-less libraries that the installer carries under `maven/`,
/// verified against the SHA-1 the profile declares. Entries that are not in
/// the jar are left alone: processors produce them (e.g. the patched client).
/// Returns how many files were written.
fn extract_embedded_libraries(
    jar: &Path,
    libs: &[EmbeddedLib],
    max_total: u64,
) -> Result<u64, LauncherError> {
    let f = std::fs::File::open(jar).map_err(LauncherError::io(jar))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| LauncherError::Extract(e.into()))?;
    let (mut written, mut count) = (0u64, 0u64);
    for lib in libs {
        let name = format!("maven/{}", lib.path);
        let Ok(mut entry) = z.by_name(&name) else {
            continue;
        };
        let Some(sha1) = lib.sha1.as_deref().map(str::to_ascii_lowercase) else {
            return Err(LauncherError::Parse {
                what: lib.path.clone(),
                message: "embedded library has no SHA-1".into(),
            });
        };
        if fsutil::sha1_file(&lib.dest).ok().as_deref() == Some(sha1.as_str()) {
            continue;
        }
        if let Some(p) = lib.dest.parent() {
            std::fs::create_dir_all(p).map_err(LauncherError::io(p))?;
        }
        let part = lib.dest.with_extension("part");
        let mut out = std::fs::File::create(&part).map_err(LauncherError::io(&part))?;
        let remaining = max_total.saturating_sub(written);
        written += std::io::copy(&mut (&mut entry).take(remaining + 1), &mut out)
            .map_err(LauncherError::io(&part))?;
        drop(out);
        if written > max_total {
            let _ = std::fs::remove_file(&part);
            return Err(LauncherError::Extract(fsutil::ExtractError::TooLarge(
                max_total,
            )));
        }
        if fsutil::sha1_file(&part).ok().as_deref() != Some(sha1.as_str()) {
            let _ = std::fs::remove_file(&part);
            return Err(LauncherError::Parse {
                what: lib.path.clone(),
                message: "embedded library hash mismatch".into(),
            });
        }
        std::fs::rename(&part, &lib.dest).map_err(LauncherError::io(&lib.dest))?;
        count += 1;
    }
    Ok(count)
}

/// Extract the named installer entries (validated relative paths) to `dest`.
/// Fails, rather than truncating, once the total exceeds `max_total`.
fn extract_entries(
    jar: &Path,
    entries: &[String],
    dest: &Path,
    max_total: u64,
) -> Result<(), LauncherError> {
    let f = std::fs::File::open(jar).map_err(LauncherError::io(jar))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| LauncherError::Extract(e.into()))?;
    let mut written = 0u64;
    for name in entries {
        let target = fsutil::safe_join(dest, name)?;
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(LauncherError::io(p))?;
        }
        let mut entry = z
            .by_name(name)
            .map_err(|e| LauncherError::Extract(e.into()))?;
        let mut out = std::fs::File::create(&target).map_err(LauncherError::io(&target))?;
        let remaining = max_total.saturating_sub(written);
        written += std::io::copy(&mut (&mut entry).take(remaining + 1), &mut out)
            .map_err(LauncherError::io(&target))?;
        if written > max_total {
            drop(out);
            let _ = std::fs::remove_file(&target);
            return Err(LauncherError::Extract(fsutil::ExtractError::TooLarge(
                max_total,
            )));
        }
    }
    Ok(())
}

pub async fn install(
    dist: Dist,
    meta: &Meta,
    key: &InstallKey,
    env: &Environment,
    cancel: &CancellationToken,
    on_progress: ProgressFn,
) -> Result<Installed, LauncherError> {
    let nf = key
        .loader_version
        .as_deref()
        .ok_or(nc_core::CoreError::MissingField {
            version: key.to_key(),
            field: "loader version",
        })?;
    let id = key.launch_id();
    let layout = &meta.layout;

    let vanilla =
        install::install_vanilla(meta, &key.minecraft, env, cancel, on_progress.clone()).await?;
    let emit = {
        let f = on_progress.clone();
        move |p: Progress| {
            f(InstallProgress {
                phase: Phase::Loader,
                progress: p,
            })
        }
    };
    emit(Progress::default());
    let was_installed = super::begin(layout, &id).await?;

    // 1. Installer jar, verified against its Maven sidecar.
    let mv = dist.maven_version(&key.minecraft, nf);
    let base = format!(
        "{}/{mv}/{}-{mv}-installer.jar",
        dist.artifact_url(meta),
        dist.slug()
    );
    let sha = String::from_utf8_lossy(&meta.dl.get_bytes(&format!("{base}.sha1"), cancel).await?)
        .trim()
        .to_ascii_lowercase();
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(LauncherError::Parse {
            what: format!("{} installer .sha1", dist.name()),
            message: "not a SHA-1".into(),
        });
    }
    let work = layout.cache_dir().join(dist.slug()).join(&mv);
    let installer = work.join("installer.jar");
    meta.dl
        .fetch_all(
            vec![DownloadTask {
                url: base,
                dest: installer.clone(),
                sha1: Some(sha),
                size: None,
                root: None,
            }],
            cancel,
            |_| {},
        )
        .await?;

    // 2. Profile + version JSON.
    let (profile, version_bytes) = {
        let inst = installer.clone();
        tokio::task::spawn_blocking(
            move || -> Result<(InstallProfile, Vec<u8>), LauncherError> {
                let p: InstallProfile =
                    serde_json::from_slice(&read_zip_entry(&inst, "install_profile.json")?)
                        .map_err(LauncherError::parse("install_profile.json"))?;
                let json_name = p.json.trim_start_matches('/').to_owned();
                nc_core::paths::validate_relative(&json_name)?;
                let v = read_zip_entry(&inst, &json_name)?;
                Ok((p, v))
            },
        )
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))??
    };
    profile.check_supported()?;
    if profile.minecraft != key.minecraft {
        return Err(LauncherError::Parse {
            what: "install_profile.json".into(),
            message: format!("is for Minecraft {}", profile.minecraft),
        });
    }
    let child: VersionJson = serde_json::from_slice(&version_bytes).map_err(
        LauncherError::parse(format!("{} version.json", dist.name())),
    )?;
    if child.id != id || child.inherits_from.as_deref() != Some(key.minecraft.as_str()) {
        return Err(LauncherError::Parse {
            what: format!("{} version.json", dist.name()),
            message: format!("unexpected id {}", child.id),
        });
    }

    // 3. Libraries (installer tools + runtime libraries).
    let mut tasks = Vec::new();
    let mut embedded = Vec::new();
    let profile_libs = VersionJson {
        libraries: profile.libraries.clone(),
        ..child.clone()
    };
    // resolve_all: the installer list has several versions of some artifacts
    // (e.g. Gson 2.8.9 for one processor, 2.10.1 for another).
    for lib in libraries::resolve_all(&profile_libs, env)?
        .into_iter()
        .chain(libraries::resolve(&child, env)?)
    {
        let dest = fsutil::safe_join(&layout.libraries_dir(), &lib.path)?;
        match lib.url {
            Some(url) => tasks.push(DownloadTask {
                url,
                dest,
                sha1: lib.sha1,
                size: lib.size,
                root: None,
            }),
            None => embedded.push(EmbeddedLib {
                path: lib.path,
                dest,
                sha1: lib.sha1,
            }),
        }
    }
    let files = meta.dl.fetch_all(tasks, cancel, emit).await?;
    if !embedded.is_empty() {
        let (inst, libs) = (installer.clone(), embedded);
        let n = tokio::task::spawn_blocking(move || {
            extract_embedded_libraries(&inst, &libs, MAX_INSTALLER_EXTRACT)
        })
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))??;
        tracing::debug!(count = n, "embedded installer libraries extracted");
    }

    // 4. Embedded data files.
    let extracted = work.join("data");
    {
        let (inst, entries, dest) = (
            installer.clone(),
            profile.embedded_files()?,
            extracted.clone(),
        );
        tokio::task::spawn_blocking(move || {
            extract_entries(&inst, &entries, &dest, MAX_INSTALLER_EXTRACT)
        })
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))??;
    }

    let ctx = ResolveContext {
        libraries_dir: &layout.libraries_dir(),
        extracted_dir: &extracted,
        minecraft_jar: &vanilla.client_jar,
        installer_jar: &installer,
        root: layout.root(),
        minecraft_version: &key.minecraft,
    };
    let data = profile.client_data(&ctx)?;

    // 5./6. Processors.
    let processors: Vec<_> = profile
        .processors
        .iter()
        .filter(|p| p.runs_on_client())
        .collect();
    let total = processors.len() as u64;
    let processor_java = if profile.spec == 0 && !processors.is_empty() {
        let e = on_progress.clone();
        crate::java::ensure_runtime(
            meta,
            SPEC0_PROCESSOR_RUNTIME,
            cancel,
            std::sync::Arc::new(move |p| {
                e(InstallProgress {
                    phase: Phase::Runtime,
                    progress: p,
                })
            }),
        )
        .await?
    } else {
        vanilla.java.clone()
    };
    for (i, proc_) in processors.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(LauncherError::Cancelled);
        }
        on_progress(InstallProgress {
            phase: Phase::Loader,
            progress: Progress {
                total_files: total,
                done_files: i as u64,
                ..Default::default()
            },
        });

        if proc_.task() == Some("DOWNLOAD_MOJMAPS") {
            let out_key = proc_
                .args
                .iter()
                .position(|a| a == "--output")
                .and_then(|j| proc_.args.get(j + 1))
                .ok_or(nc_core::CoreError::MissingField {
                    version: id.clone(),
                    field: "DOWNLOAD_MOJMAPS --output",
                })?;
            let dest = PathBuf::from(resolve_arg(out_key, &data, &layout.libraries_dir())?);
            let maps = vanilla
                .version
                .downloads
                .as_ref()
                .and_then(|d| d.client_mappings.clone())
                .ok_or(nc_core::CoreError::MissingField {
                    version: key.minecraft.clone(),
                    field: "downloads.client_mappings",
                })?;
            meta.dl
                .fetch_all(
                    vec![DownloadTask {
                        url: maps.url,
                        dest,
                        sha1: Some(maps.sha1),
                        size: Some(maps.size),
                        root: None,
                    }],
                    cancel,
                    |_| {},
                )
                .await?;
            continue;
        }

        // Declared outputs, handled as the official installer does: when all
        // are already valid the processor is skipped; otherwise stale files
        // are deleted first, because some tools (jarsplitter) keep a cache
        // and would otherwise leave a bad output in place.
        let outputs = proc_
            .outputs
            .iter()
            .map(|(k, v)| {
                Ok::<_, LauncherError>((
                    PathBuf::from(resolve_arg(k, &data, &layout.libraries_dir())?),
                    resolve_arg(v, &data, &layout.libraries_dir())?
                        .trim_matches('\'')
                        .to_ascii_lowercase(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !outputs.is_empty() {
            let stale = invalid_outputs(&outputs).await?;
            if stale.is_empty() {
                continue;
            }
            for p in stale {
                let _ = tokio::fs::remove_file(&p).await;
            }
        }

        let jar = maven_path(&layout.libraries_dir(), &proc_.jar)?;
        let manifest = {
            let j = jar.clone();
            tokio::task::spawn_blocking(move || read_zip_entry(&j, "META-INF/MANIFEST.MF"))
                .await
                .map_err(|e| LauncherError::Spawn(e.to_string()))??
        };
        let main = main_class(&String::from_utf8_lossy(&manifest)).ok_or_else(|| {
            LauncherError::Parse {
                what: proc_.jar.clone(),
                message: "no Main-Class".into(),
            }
        })?;
        let mut cp = vec![jar.display().to_string()];
        for c in &proc_.classpath {
            cp.push(
                maven_path(&layout.libraries_dir(), c)?
                    .display()
                    .to_string(),
            );
        }
        let args = proc_
            .args
            .iter()
            .map(|a| resolve_arg(a, &data, &layout.libraries_dir()))
            .collect::<Result<Vec<_>, _>>()?;
        run_processor(&processor_java, &cp.join(";"), &main, &args, &work, cancel)
            .await
            .map_err(|e| match e {
                LauncherError::Spawn(m) => {
                    LauncherError::Processor(format!("{}: {m}", proc_.task().unwrap_or(&proc_.jar)))
                }
                other => other,
            })?;

        if let Some(bad) = invalid_outputs(&outputs).await?.into_iter().next() {
            return Err(LauncherError::Parse {
                what: bad.display().to_string(),
                message: "processor output hash mismatch".into(),
            });
        }
    }
    on_progress(InstallProgress {
        phase: Phase::Loader,
        progress: Progress {
            total_files: total,
            done_files: total,
            ..Default::default()
        },
    });

    if let Some(patched) = data.get("PATCHED")
        && !Path::new(patched).is_file()
    {
        return Err(LauncherError::FilesMissing(format!(
            "patched client {patched}"
        )));
    }
    // Every runtime library must now exist: downloaded, taken from the
    // installer, or written by a processor.
    for lib in libraries::resolve(&child, env)? {
        let p = fsutil::safe_join(&layout.libraries_dir(), &lib.path)?;
        if !p.is_file() {
            return Err(LauncherError::FilesMissing(format!("library {}", lib.name)));
        }
    }

    super::finish(layout, &id, &version_bytes, &vanilla.client_jar).await?;
    let _ = tokio::fs::remove_dir_all(&extracted).await;
    let merged = super::resolve_version(layout, &id).await?;
    let libs = libraries::resolve(&merged, env)?;
    let mut repaired = vanilla.repaired.clone();
    if was_installed {
        repaired.extend(super::relative(layout.root(), &files.fetched));
    }
    tracing::info!(%id, loader = dist.name(), processors = total, "loader installed");
    Ok(Installed {
        version: merged,
        libraries: libs,
        java: vanilla.java.clone(),
        client_jar: layout.client_jar(&id)?,
        log_config: vanilla.log_config.clone(),
        files: Summary {
            downloaded_files: vanilla.files.downloaded_files + files.downloaded_files,
            skipped_files: vanilla.files.skipped_files + files.skipped_files,
            downloaded_bytes: vanilla.files.downloaded_bytes + files.downloaded_bytes,
            fetched: Vec::new(),
        },
        was_installed: was_installed || vanilla.was_installed,
        repaired,
        repaired_count: vanilla.repaired_count
            + if was_installed {
                files.downloaded_files
            } else {
                0
            },
    })
}

/// Paths among `outputs` that are missing or whose SHA-1 is not the expected one.
async fn invalid_outputs(outputs: &[(PathBuf, String)]) -> Result<Vec<PathBuf>, LauncherError> {
    let list = outputs.to_vec();
    tokio::task::spawn_blocking(move || {
        list.into_iter()
            .filter(|(p, sha)| fsutil::sha1_file(p).ok().as_deref() != Some(sha.as_str()))
            .map(|(p, _)| p)
            .collect()
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))
}

/// Run one processor; on failure, return the last lines of its output.
async fn run_processor(
    java_w: &Path,
    classpath: &str,
    main: &str,
    args: &[String],
    cwd: &Path,
    cancel: &CancellationToken,
) -> Result<(), LauncherError> {
    // Processors are console programs: use java.exe (not javaw) to get output.
    let java = java_w.with_file_name("java.exe");
    let mut cmd = tokio::process::Command::new(&java);
    cmd.arg("-cp")
        .arg(classpath)
        .arg(main)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    tracing::info!(%main, "running NeoForge processor");
    let mut child = cmd
        .spawn()
        .map_err(|e| LauncherError::Spawn(e.to_string()))?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let collect = async {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut buf).await;
        }
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut buf).await;
        }
        buf
    };
    let (status, output) = tokio::select! {
        r = async { let o = collect.await; (child.wait().await, o) } => r,
        _ = cancel.cancelled() => { let _ = child.kill().await; return Err(LauncherError::Cancelled); }
        _ = tokio::time::sleep(PROCESSOR_TIMEOUT) => { let _ = child.kill().await; return Err(LauncherError::Spawn("timed out after 10 minutes".into())); }
    };
    let status = status.map_err(|e| LauncherError::Spawn(e.to_string()))?;
    if status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output);
    let tail: Vec<&str> = text
        .lines()
        .rev()
        .take(15)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Err(LauncherError::Spawn(format!(
        "exit code {:?}: {}",
        status.code(),
        tail.join(" | ")
    )))
}

#[cfg(test)]
mod tests {
    use super::main_class;

    #[test]
    fn reads_main_class_with_continuations() {
        assert_eq!(main_class("Manifest-Version: 1.0\r\nMain-Class: net.minecraftforge.binarypatcher.ConsoleTool\r\n").as_deref(), Some("net.minecraftforge.binarypatcher.ConsoleTool"));
        assert_eq!(
            main_class("Main-Class: net.neoforged.installertools.Insta\n llerTools\n").as_deref(),
            Some("net.neoforged.installertools.InstallerTools")
        );
        assert_eq!(main_class("Manifest-Version: 1.0\n"), None);
    }

    fn jar(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut w = zip::ZipWriter::new(f.reopen().unwrap());
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        f
    }

    #[test]
    fn extracts_named_entries_and_fails_instead_of_truncating() {
        let j = jar(&[
            ("data/client.lzma", &[7u8; 600]),
            ("data/server.lzma", &[1u8; 600]),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let names = ["data/client.lzma".to_owned()];
        super::extract_entries(j.path(), &names, dir.path(), 1 << 20).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("data/client.lzma"))
                .unwrap()
                .len(),
            600
        );
        assert!(!dir.path().join("data/server.lzma").exists());

        // Over the limit: an error, and no partial file left behind.
        let out = tempfile::tempdir().unwrap();
        let both = ["data/client.lzma".to_owned(), "data/server.lzma".to_owned()];
        let err = super::extract_entries(j.path(), &both, out.path(), 1000).unwrap_err();
        assert!(err.to_string().contains("extraction limit"), "{err}");
        assert!(!out.path().join("data/server.lzma").exists());
    }

    #[test]
    fn extracts_embedded_libraries_verified_and_skips_processor_outputs() {
        use super::{EmbeddedLib, extract_embedded_libraries};
        let body = b"forge universal";
        let sha = {
            let t = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(t.path(), body).unwrap();
            crate::fsutil::sha1_file(t.path()).unwrap()
        };
        let j = jar(&[("maven/net/f/forge/1/forge-1-universal.jar", body)]);
        let dir = tempfile::tempdir().unwrap();
        let lib = |path: &str, sha1: Option<&str>| EmbeddedLib {
            path: path.into(),
            dest: dir.path().join(path),
            sha1: sha1.map(str::to_owned),
        };
        let uni = lib("net/f/forge/1/forge-1-universal.jar", Some(&sha));
        // Not in the jar: a processor writes it later, so it is skipped.
        let client = lib("net/f/forge/1/forge-1-client.jar", Some(&sha));
        let libs = [uni.clone(), client.clone()];
        assert_eq!(
            extract_embedded_libraries(j.path(), &libs, 1 << 20).unwrap(),
            1
        );
        assert_eq!(std::fs::read(&uni.dest).unwrap(), body);
        assert!(!client.dest.exists());
        // Already present and valid: nothing rewritten.
        assert_eq!(
            extract_embedded_libraries(j.path(), &libs, 1 << 20).unwrap(),
            0
        );

        // Wrong declared hash: refused, and nothing left behind.
        std::fs::remove_file(&uni.dest).unwrap();
        let bad = lib(&uni.path, Some("0000000000000000000000000000000000000000"));
        let err =
            extract_embedded_libraries(j.path(), std::slice::from_ref(&bad), 1 << 20).unwrap_err();
        assert!(err.to_string().contains("hash mismatch"), "{err}");
        assert!(!bad.dest.exists() && !bad.dest.with_extension("part").exists());
        // No declared hash: refused.
        assert!(extract_embedded_libraries(j.path(), &[lib(&uni.path, None)], 1 << 20).is_err());
        // Over the size limit: refused.
        assert!(extract_embedded_libraries(j.path(), &[uni], 4).is_err());
    }

    #[tokio::test]
    async fn reports_missing_and_stale_processor_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let (good, stale, missing) = (
            dir.path().join("good.jar"),
            dir.path().join("stale.jar"),
            dir.path().join("missing.jar"),
        );
        std::fs::write(&good, b"a").unwrap();
        std::fs::write(&stale, b"b").unwrap();
        let sha_a = crate::fsutil::sha1_file(&good).unwrap();
        let outputs = [
            (good.clone(), sha_a.clone()),
            (stale.clone(), sha_a.clone()),
            (missing.clone(), sha_a),
        ];
        assert_eq!(
            super::invalid_outputs(&outputs).await.unwrap(),
            [stale, missing]
        );
        assert!(
            super::invalid_outputs(&outputs[..1])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn refuses_entry_names_that_escape_the_destination() {
        let j = jar(&[("../evil.txt", b"x")]);
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("work");
        let err = super::extract_entries(j.path(), &["../evil.txt".to_owned()], &sub, 1 << 20);
        assert!(err.is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }
}
