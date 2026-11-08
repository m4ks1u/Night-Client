//! Discovery and inspection of Java runtimes installed on the machine.
//!
//! Inspection prefers static facts — the JDK `release` file and the PE header
//! of `java.exe` — so scanning never has to execute unknown binaries. Only a
//! runtime without a `release` file is asked for `-version` (with a timeout,
//! no console window, no shell).

use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use nc_core::java::{PeArch, parse_major, parse_release_file, parse_version_output, pe_arch};
use serde::Serialize;

use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JavaSource {
    /// Installed by Night Client from Mojang.
    Managed,
    JavaHome,
    Path,
    ProgramFiles,
    /// Recorded in the Windows registry by a vendor installer.
    Registry,
    /// Bundled with the official Minecraft Launcher.
    MinecraftLauncher,
    /// Chosen by the user.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    /// `…\bin\javaw.exe`: what the launcher starts.
    pub path: PathBuf,
    pub home: PathBuf,
    pub version: String,
    pub major: u32,
    pub vendor: Option<String>,
    pub arch: PeArch,
    pub source: JavaSource,
}

#[derive(Debug, thiserror::Error)]
pub enum JavaError {
    #[error("{0} is not a Java executable (expected java.exe or javaw.exe in a bin folder)")]
    NotJava(PathBuf),
    #[error("could not determine the Java version of {0}")]
    UnknownVersion(PathBuf),
}

/// Resolve a user-supplied path (java.exe, javaw.exe, bin folder or Java
/// home) to a home directory containing `bin\javaw.exe`.
pub fn home_of(input: &Path) -> Option<PathBuf> {
    let candidates = [
        input.to_path_buf(),
        input.parent().map(Path::to_path_buf).unwrap_or_default(), // …\bin
        input
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default(), // home
    ];
    candidates.into_iter().find(|h| {
        h.join("bin").join("javaw.exe").is_file() && h.join("bin").join("java.exe").is_file()
    })
}

pub fn inspect(input: &Path, source: JavaSource) -> Result<JavaRuntime, JavaError> {
    if input.is_file() {
        let name = input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name != "java.exe" && name != "javaw.exe" {
            return Err(JavaError::NotJava(input.into()));
        }
    }
    let home = home_of(input).ok_or_else(|| JavaError::NotJava(input.into()))?;
    let java = home.join("bin").join("java.exe");
    let release = std::fs::read_to_string(home.join("release"))
        .map(|t| parse_release_file(&t))
        .unwrap_or_default();
    let version = match release.java_version.clone() {
        Some(v) => v,
        None => query_version(&java).ok_or_else(|| JavaError::UnknownVersion(home.clone()))?,
    };
    let major = parse_major(&version).ok_or_else(|| JavaError::UnknownVersion(home.clone()))?;
    Ok(JavaRuntime {
        path: home.join("bin").join("javaw.exe"),
        arch: read_arch(&java),
        vendor: release.implementor,
        version,
        major,
        home,
        source,
    })
}

fn read_arch(exe: &Path) -> PeArch {
    let mut buf = vec![0u8; 4096];
    match std::fs::File::open(exe).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => pe_arch(&buf[..n]),
        Err(_) => PeArch::Unknown,
    }
}

/// `java -version` with a 5 s timeout.
fn query_version(java: &Path) -> Option<String> {
    let mut cmd = Command::new(java);
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    cmd.env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut err = String::new();
    child
        .stderr
        .take()?
        .take(64 * 1024)
        .read_to_string(&mut err)
        .ok()?;
    parse_version_output(&err)
}

/// Directories (up to `depth` levels below `root`) that look like Java homes.
fn find_homes(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if root.join("bin").join("javaw.exe").is_file() {
        out.push(root.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            find_homes(&e.path(), depth - 1, out);
        }
    }
}

/// Scan well-known locations. Results are de-duplicated by home directory
/// and sorted by major version, newest first.
pub fn detect(layout: &Layout) -> Vec<JavaRuntime> {
    let mut candidates: Vec<(PathBuf, JavaSource)> = Vec::new();
    let mut push_all = |root: &Path, depth: usize, source: JavaSource| {
        let mut homes = Vec::new();
        find_homes(root, depth, &mut homes);
        candidates.extend(homes.into_iter().map(|h| (h, source)));
    };

    push_all(&layout.runtimes_dir(), 1, JavaSource::Managed);
    if let Some(h) = std::env::var_os("JAVA_HOME") {
        push_all(Path::new(&h), 0, JavaSource::JavaHome);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("java.exe").is_file()
                && let Some(home) = dir.parent()
            {
                push_all(home, 0, JavaSource::Path);
            }
        }
    }
    for var in ["ProgramFiles", "ProgramW6432"] {
        if let Some(pf) = std::env::var_os(var) {
            for vendor in [
                "Java",
                "Eclipse Adoptium",
                "AdoptOpenJDK",
                "Microsoft",
                "Zulu",
                "BellSoft",
                "Amazon Corretto",
                "Semeru",
                "OpenJDK",
                "Eclipse Foundation",
                "GraalVM",
            ] {
                push_all(&Path::new(&pf).join(vendor), 2, JavaSource::ProgramFiles);
            }
        }
    }
    // Installs outside the standard folders (custom location, other drive).
    // Earlier sources win the de-duplication below, so a runtime already
    // found under Program Files keeps that label.
    #[cfg(windows)]
    for home in crate::javareg::homes(crate::javareg::VENDOR_ROOTS) {
        push_all(&home, 0, JavaSource::Registry);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let store = Path::new(&local)
            .join(r"Packages\Microsoft.4297127D64EC6_8wekyb3d8bbwe\LocalCache\Local\runtime");
        push_all(&store, 3, JavaSource::MinecraftLauncher);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        push_all(
            &Path::new(&appdata).join(r".minecraft\runtime"),
            3,
            JavaSource::MinecraftLauncher,
        );
    }
    if let Some(pf86) = std::env::var_os("ProgramFiles(x86)") {
        push_all(
            &Path::new(&pf86).join(r"Minecraft Launcher\runtime"),
            3,
            JavaSource::MinecraftLauncher,
        );
    }

    let mut seen = HashSet::new();
    let mut found: Vec<JavaRuntime> = candidates
        .into_iter()
        .filter(|(h, _)| seen.insert(std::fs::canonicalize(h).unwrap_or_else(|_| h.clone())))
        .filter_map(|(h, s)| {
            inspect(&h, s)
                .inspect_err(|e| tracing::debug!(error = %e, "skipping Java candidate"))
                .ok()
        })
        .collect();
    found.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.path.cmp(&b.path)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home(root: &Path, release: &str) -> PathBuf {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut pe = vec![0u8; 0x100];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        std::fs::write(bin.join("java.exe"), &pe).unwrap();
        std::fs::write(bin.join("javaw.exe"), &pe).unwrap();
        std::fs::write(root.join("release"), release).unwrap();
        root.to_path_buf()
    }

    #[test]
    fn inspects_without_executing_when_release_exists() {
        let d = tempfile::tempdir().unwrap();
        let home = fake_home(
            &d.path().join("jdk-21"),
            "JAVA_VERSION=\"21.0.7\"\nIMPLEMENTOR=\"Microsoft\"\n",
        );
        for input in [
            home.clone(),
            home.join("bin"),
            home.join("bin").join("javaw.exe"),
            home.join("bin").join("java.exe"),
        ] {
            let r = inspect(&input, JavaSource::Custom).unwrap();
            assert_eq!(
                (r.major, r.arch, r.vendor.as_deref()),
                (21, PeArch::X64, Some("Microsoft"))
            );
            assert_eq!(r.path, home.join("bin").join("javaw.exe"));
        }
    }

    #[test]
    fn rejects_non_java_paths() {
        let d = tempfile::tempdir().unwrap();
        let home = fake_home(&d.path().join("jdk"), "JAVA_VERSION=\"17\"");
        std::fs::write(home.join("bin").join("evil.exe"), b"x").unwrap();
        assert!(matches!(
            inspect(&home.join("bin").join("evil.exe"), JavaSource::Custom),
            Err(JavaError::NotJava(_))
        ));
        assert!(matches!(
            inspect(d.path(), JavaSource::Custom),
            Err(JavaError::NotJava(_))
        ));
    }

    #[test]
    fn finds_nested_homes_and_dedups() {
        let d = tempfile::tempdir().unwrap();
        fake_home(
            &d.path().join("Zulu").join("zulu-17"),
            "JAVA_VERSION=\"17.0.9\"",
        );
        fake_home(
            &d.path().join("Microsoft").join("jdk-21.0.7.6-hotspot"),
            "JAVA_VERSION=\"21.0.7\"",
        );
        let mut homes = Vec::new();
        find_homes(d.path(), 2, &mut homes);
        assert_eq!(homes.len(), 2);
    }

    #[test]
    fn detects_real_managed_runtimes_if_present() {
        // Uses the real data directory; passes trivially on machines without runtimes.
        let layout = Layout::new(Layout::default_root());
        for r in detect(&layout) {
            assert!(r.path.is_file());
            assert!(r.major >= 8);
        }
    }
}
