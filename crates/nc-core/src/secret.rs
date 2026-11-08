use std::fmt;

use zeroize::Zeroize;

/// A string that must never be logged or serialized by accident.
///
/// `Debug` and `Display` print a fixed placeholder. The only way to read the
/// value is [`Secret::expose`], which makes every use greppable in review.
/// The buffer is overwritten with zeros when the value is dropped, so a
/// token does not linger in freed heap memory. (Copies made by HTTP and JSON
/// libraries before the value reaches a `Secret` are outside our control.)
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Replace every occurrence of each secret in `text` with `<redacted>`.
pub fn redact(text: &str, secrets: &[&Secret]) -> String {
    let mut out = text.to_owned();
    for s in secrets {
        if s.expose().len() >= 4 {
            out = out.replace(s.expose(), "<redacted>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_leak() {
        let s = Secret::new("eyJhbGciOi-super-secret");
        assert!(!format!("{s:?}").contains("super-secret"));
        assert!(!format!("{s}").contains("super-secret"));
    }

    #[test]
    fn redact_replaces_all_occurrences() {
        let s = Secret::new("tok123");
        assert_eq!(
            redact("a tok123 b tok123", &[&s]),
            "a <redacted> b <redacted>"
        );
    }
}
