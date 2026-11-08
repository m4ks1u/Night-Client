//! Validation of relative paths that come from untrusted metadata
//! (version JSON, loader profiles, runtime manifests, mod metadata).

use crate::CoreError;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate a `/`-separated relative path from metadata.
///
/// Rejects absolute paths, drive letters, UNC prefixes, `..` and `.`
/// segments, backslashes, NTFS alternate data streams (`:`), control
/// characters, Windows reserved device names and trailing dots/spaces
/// (which Windows silently strips, enabling aliasing).
pub fn validate_relative(path: &str) -> Result<(), CoreError> {
    let bad = || Err(CoreError::UnsafePath(path.to_owned()));
    if path.is_empty() || path.len() > 1024 || path.starts_with('/') || path.contains('\\') {
        return bad();
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return bad();
        }
        if seg
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        {
            return bad();
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return bad();
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_uppercase();
        if RESERVED.contains(&stem.as_str()) {
            return bad();
        }
    }
    Ok(())
}

/// Validate a single path segment (file or directory name).
pub fn validate_segment(seg: &str) -> Result<(), CoreError> {
    if seg.contains('/') {
        return Err(CoreError::UnsafePath(seg.to_owned()));
    }
    validate_relative(seg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_paths() {
        for p in [
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar",
            "bin/javaw.exe",
            "a.b-c_d/e f.txt",
        ] {
            assert!(validate_relative(p).is_ok(), "{p}");
        }
    }

    #[test]
    fn rejects_traversal_and_windows_tricks() {
        for p in [
            "",
            "../evil.jar",
            "a/../../evil",
            "a/./b",
            "/etc/passwd",
            "C:/Windows/System32",
            "C:evil",
            "a\\..\\b",
            "\\\\server\\share",
            "file.jar:stream",
            "a//b",
            "CON",
            "dir/nul.txt",
            "trailing./x",
            "space /x",
            "bad\u{0}name",
        ] {
            assert!(validate_relative(p).is_err(), "should reject {p:?}");
        }
    }
}
