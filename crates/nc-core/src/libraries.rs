use std::collections::HashSet;

use crate::{
    CoreError,
    manifest::{Artifact, Library, VersionJson},
    maven::MavenCoord,
    paths::validate_relative,
    rules::{Environment, rules_allow},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryKind {
    /// Goes on the classpath.
    Classpath,
    /// Legacy natives jar (pre-1.19): extracted into the natives directory.
    Native { exclude: Vec<String> },
}

/// A library file to download, after rules have been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLibrary {
    pub name: String,
    /// Validated, `/`-separated path relative to the libraries directory.
    pub path: String,
    /// `None` when the file is produced locally (e.g. by a loader installer).
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub kind: LibraryKind,
}

/// Resolve which library files a version needs on `env`.
///
/// Classpath entries are de-duplicated by `group:artifact[:classifier]`,
/// keeping the first occurrence (child profiles come first after merging).
pub fn resolve(
    version: &VersionJson,
    env: &Environment,
) -> Result<Vec<ResolvedLibrary>, CoreError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for lib in version
        .libraries
        .iter()
        .filter(|l| rules_allow(&l.rules, env))
    {
        let coord = MavenCoord::parse(&lib.name)?;

        if let Some(classifier) = native_classifier(lib, env) {
            let native = coord.with_classifier(&classifier)?;
            let artifact = lib
                .downloads
                .as_ref()
                .and_then(|d| d.classifiers.as_ref())
                .and_then(|c| c.get(&classifier));
            let exclude = lib
                .extract
                .as_ref()
                .map(|e| e.exclude.clone())
                .unwrap_or_default();
            out.push(from_artifact(
                lib,
                &native,
                artifact,
                LibraryKind::Native { exclude },
            )?);
        }

        let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
        let has_classpath_jar = artifact.is_some() || lib.downloads.is_none();
        if has_classpath_jar && seen.insert(coord.key()) {
            out.push(from_artifact(
                lib,
                &coord,
                artifact,
                LibraryKind::Classpath,
            )?);
        }
    }
    Ok(out)
}

fn native_classifier(lib: &Library, env: &Environment) -> Option<String> {
    lib.natives
        .as_ref()?
        .get(env.os.as_str())
        .map(|c| c.replace("${arch}", env.arch.bits()))
}

fn from_artifact(
    lib: &Library,
    coord: &MavenCoord,
    artifact: Option<&Artifact>,
    kind: LibraryKind,
) -> Result<ResolvedLibrary, CoreError> {
    let path = artifact
        .and_then(|a| a.path.clone())
        .unwrap_or_else(|| coord.path());
    validate_relative(&path)?;

    let (url, sha1, size) = match artifact {
        Some(a) => (
            (!a.url.is_empty()).then(|| a.url.clone()),
            a.sha1.clone(),
            a.size,
        ),
        // Fabric-style: Maven repository base + coordinate path.
        None => (
            lib.url.as_ref().map(|base| {
                let base = base.trim_end_matches('/');
                format!("{base}/{}", coord.path())
            }),
            lib.sha1.clone(),
            lib.size,
        ),
    };
    if let Some(u) = &url
        && !u.starts_with("https://")
    {
        return Err(CoreError::UnsafePath(u.clone()));
    }

    Ok(ResolvedLibrary {
        name: lib.name.clone(),
        path,
        url,
        sha1,
        size,
        kind,
    })
}
