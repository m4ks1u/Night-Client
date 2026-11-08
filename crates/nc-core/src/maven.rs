use crate::{CoreError, paths::validate_segment};

/// A Maven coordinate: `group:artifact:version[:classifier][@extension]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoord {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoord {
    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let err = || CoreError::InvalidCoordinate(s.to_owned());
        let (body, extension) = match s.split_once('@') {
            Some((b, e)) => (b, e.to_owned()),
            None => (s, "jar".to_owned()),
        };
        let parts: Vec<&str> = body.split(':').collect();
        let (group, artifact, version, classifier) = match parts.as_slice() {
            [g, a, v] => (*g, *a, *v, None),
            [g, a, v, c] => (*g, *a, *v, Some((*c).to_owned())),
            _ => return Err(err()),
        };
        // Every component becomes a path segment, so validate each one.
        for seg in group
            .split('.')
            .chain([artifact, version, extension.as_str()])
        {
            validate_segment(seg).map_err(|_| err())?;
        }
        if let Some(c) = &classifier {
            validate_segment(c).map_err(|_| err())?;
        }
        Ok(Self {
            group: group.to_owned(),
            artifact: artifact.to_owned(),
            version: version.to_owned(),
            classifier,
            extension,
        })
    }

    pub fn with_classifier(&self, classifier: &str) -> Result<Self, CoreError> {
        validate_segment(classifier)?;
        Ok(Self {
            classifier: Some(classifier.to_owned()),
            ..self.clone()
        })
    }

    /// Repository-relative path, always `/`-separated.
    pub fn path(&self) -> String {
        let file = match &self.classifier {
            Some(c) => format!(
                "{}-{}-{}.{}",
                self.artifact, self.version, c, self.extension
            ),
            None => format!("{}-{}.{}", self.artifact, self.version, self.extension),
        };
        format!(
            "{}/{}/{}/{}",
            self.group.replace('.', "/"),
            self.artifact,
            self.version,
            file
        )
    }

    /// Identity used to de-duplicate libraries regardless of version.
    pub fn key(&self) -> String {
        match &self.classifier {
            Some(c) => format!("{}:{}:{}", self.group, self.artifact, c),
            None => format!("{}:{}", self.group, self.artifact),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_builds_paths() {
        let c = MavenCoord::parse("org.lwjgl:lwjgl:3.3.3:natives-windows").unwrap();
        assert_eq!(
            c.path(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(c.key(), "org.lwjgl:lwjgl:natives-windows");
        let z = MavenCoord::parse("net.neoforged:neoform:1.21.1-20240808.144430@zip").unwrap();
        assert_eq!(z.extension, "zip");
        assert!(z.path().ends_with("neoform-1.21.1-20240808.144430.zip"));
    }

    #[test]
    fn rejects_malicious_coordinates() {
        for s in [
            "a:b",
            "a:b:c:d:e",
            "..:b:1",
            "a:..:1",
            "a:b:../../x",
            "a:b:1:../x",
            "a:b:1@../exe",
            "a..b:c:1",
            "a:b\\c:1",
        ] {
            assert!(MavenCoord::parse(s).is_err(), "should reject {s}");
        }
    }
}
