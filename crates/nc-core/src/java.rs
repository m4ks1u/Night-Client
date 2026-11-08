use serde::Serialize;

use crate::manifest::{JavaVersionReq, VersionJson};

/// The Java runtime a version needs. Versions published before Mojang added
/// `javaVersion` (pre-1.17 era files that were never re-published) need Java 8.
pub fn requirement(version: &VersionJson) -> JavaVersionReq {
    version.java_version.clone().unwrap_or(JavaVersionReq {
        component: "jre-legacy".into(),
        major_version: 8,
    })
}

/// Parse the major version from `java -version` style strings
/// (`1.8.0_412`, `17.0.2`, `21`, `25-ea`).
pub fn parse_major(version: &str) -> Option<u32> {
    let mut parts = version.split(|c: char| !c.is_ascii_digit());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// Fields read from a JDK/JRE `release` file (`KEY="value"` lines).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReleaseFile {
    pub java_version: Option<String>,
    pub implementor: Option<String>,
    pub os_arch: Option<String>,
}

pub fn parse_release_file(text: &str) -> ReleaseFile {
    let mut r = ReleaseFile::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').to_owned();
        match k.trim() {
            "JAVA_VERSION" => r.java_version = Some(v),
            "IMPLEMENTOR" => r.implementor = Some(v),
            "OS_ARCH" => r.os_arch = Some(v),
            _ => {}
        }
    }
    r
}

/// Extract the version from `java -version` output, e.g.
/// `openjdk version "21.0.7" 2025-04-15 LTS`.
pub fn parse_version_output(output: &str) -> Option<String> {
    let line = output.lines().find(|l| l.contains(" version \""))?;
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Compatibility {
    /// Exactly the major version the game declares.
    Recommended,
    /// Newer than required, for versions (Java 17+) that tolerate it.
    Compatible,
    /// Newer than required for an old game that expects Java 8/16:
    /// often works, sometimes crashes (mods especially).
    Risky,
    /// Too old, or not a 64-bit runtime.
    Incompatible,
}

/// Judge a runtime against a version's requirement. Night Client only
/// supports x64 Windows, so 32-bit or ARM runtimes are incompatible.
pub fn compatibility(required_major: u32, found_major: u32, is_x64: bool) -> Compatibility {
    if !is_x64 || found_major < required_major {
        Compatibility::Incompatible
    } else if found_major == required_major {
        Compatibility::Recommended
    } else if required_major >= 17 {
        Compatibility::Compatible
    } else {
        Compatibility::Risky
    }
}

/// CPU architecture from a Windows PE header (`IMAGE_FILE_HEADER.Machine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeArch {
    X64,
    X86,
    Arm64,
    Unknown,
}

pub fn pe_arch(header: &[u8]) -> PeArch {
    let read_u32 = |o: usize| {
        header
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let read_u16 = |o: usize| {
        header
            .get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    if header.get(..2) != Some(b"MZ") {
        return PeArch::Unknown;
    }
    let Some(pe) = read_u32(0x3C).map(|v| v as usize) else {
        return PeArch::Unknown;
    };
    if header.get(pe..pe + 4) != Some(b"PE\0\0") {
        return PeArch::Unknown;
    }
    match read_u16(pe + 4) {
        Some(0x8664) => PeArch::X64,
        Some(0x014c) => PeArch::X86,
        Some(0xAA64) => PeArch::Arm64,
        _ => PeArch::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_major("1.8.0_412"), Some(8));
        assert_eq!(parse_major("17.0.2"), Some(17));
        assert_eq!(parse_major("21"), Some(21));
        assert_eq!(parse_major("25-ea"), Some(25));
        assert_eq!(parse_major("abc"), None);
    }

    #[test]
    fn parses_release_files() {
        let r = parse_release_file(
            "IMPLEMENTOR=\"Eclipse Adoptium\"\nJAVA_VERSION=\"21.0.7\"\nOS_ARCH=\"amd64\"\nMODULES=\"java.base\"\n",
        );
        assert_eq!(r.java_version.as_deref(), Some("21.0.7"));
        assert_eq!(r.implementor.as_deref(), Some("Eclipse Adoptium"));
        assert_eq!(r.os_arch.as_deref(), Some("amd64"));
        assert_eq!(parse_release_file("garbage"), ReleaseFile::default());
    }

    #[test]
    fn parses_version_output() {
        let out = "openjdk version \"21.0.7\" 2025-04-15 LTS\nOpenJDK Runtime Environment Microsoft-11369940 (build 21.0.7+6-LTS)";
        assert_eq!(parse_version_output(out).as_deref(), Some("21.0.7"));
        assert_eq!(
            parse_version_output("java version \"1.8.0_51\"").as_deref(),
            Some("1.8.0_51")
        );
        assert_eq!(parse_version_output("nope"), None);
    }

    #[test]
    fn compatibility_matrix() {
        use Compatibility::*;
        assert_eq!(compatibility(21, 21, true), Recommended);
        assert_eq!(compatibility(21, 25, true), Compatible);
        assert_eq!(compatibility(21, 17, true), Incompatible);
        assert_eq!(compatibility(8, 8, true), Recommended);
        assert_eq!(compatibility(8, 21, true), Risky);
        assert_eq!(compatibility(16, 17, true), Risky);
        assert_eq!(compatibility(8, 8, false), Incompatible);
    }

    #[test]
    fn reads_pe_machine() {
        let mut h = vec![0u8; 0x100];
        h[..2].copy_from_slice(b"MZ");
        h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        h[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        assert_eq!(pe_arch(&h), PeArch::X64);
        h[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        assert_eq!(pe_arch(&h), PeArch::X86);
        assert_eq!(pe_arch(b"not a pe"), PeArch::Unknown);
        h[0x3C..0x40].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert_eq!(pe_arch(&h), PeArch::Unknown, "out-of-range offset is safe");
    }
}
