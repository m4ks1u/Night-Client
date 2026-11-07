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
}
