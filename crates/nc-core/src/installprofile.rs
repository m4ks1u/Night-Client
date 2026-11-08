//! Forge-family `install_profile.json` (spec 1), as used by NeoForge.
//!
//! The installer jar declares data values and a list of processors (Java
//! programs) that build the patched game jar. This module only models the
//! file and resolves processor arguments; running them is the launcher's job.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{CoreError, manifest::Library, maven::MavenCoord, paths::validate_relative};

pub const SUPPORTED_SPEC: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct InstallProfile {
    pub spec: u32,
    pub minecraft: String,
    /// Path of the version JSON inside the installer (e.g. `/version.json`).
    pub json: String,
    #[serde(default)]
    pub data: BTreeMap<String, SidedValue>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub libraries: Vec<Library>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SidedValue {
    pub client: String,
    #[serde(default)]
    pub server: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Processor {
    pub jar: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub sides: Option<Vec<String>>,
    /// Expected outputs: `{ "{KEY}" or "[coord]": "'sha1'" or "{KEY}" }`.
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
}

impl Processor {
    pub fn runs_on_client(&self) -> bool {
        self.sides
            .as_ref()
            .is_none_or(|s| s.iter().any(|x| x == "client"))
    }

    /// `--task NAME` value, if this is an installertools invocation.
    pub fn task(&self) -> Option<&str> {
        let i = self.args.iter().position(|a| a == "--task")?;
        self.args.get(i + 1).map(String::as_str)
    }
}

/// Where data values that point into the installer (`/data/client.lzma`)
/// were extracted, plus the built-in tokens.
pub struct ResolveContext<'a> {
    pub libraries_dir: &'a Path,
    /// Directory the installer's `/data/...` entries were extracted to.
    pub extracted_dir: &'a Path,
    pub minecraft_jar: &'a Path,
    pub installer_jar: &'a Path,
    pub root: &'a Path,
    pub minecraft_version: &'a str,
}

impl InstallProfile {
    pub fn check_supported(&self) -> Result<(), CoreError> {
        if self.spec == SUPPORTED_SPEC {
            Ok(())
        } else {
            Err(CoreError::MissingField {
                version: format!("install_profile spec {}", self.spec),
                field: "supported spec (1)",
            })
        }
    }

    /// Installer-relative paths (`data/client.lzma`) that must be extracted.
    pub fn embedded_files(&self) -> Result<Vec<String>, CoreError> {
        let mut out = Vec::new();
        for v in self.data.values() {
            if let Some(rel) = v.client.strip_prefix('/') {
                validate_relative(rel)?;
                out.push(rel.to_owned());
            }
        }
        Ok(out)
    }

    /// Resolve client-side data values to concrete strings/paths.
    pub fn client_data(
        &self,
        ctx: &ResolveContext<'_>,
    ) -> Result<HashMap<String, String>, CoreError> {
        let d = |p: &Path| p.display().to_string();
        let mut map = HashMap::from([
            ("SIDE".to_owned(), "client".to_owned()),
            ("MINECRAFT_JAR".to_owned(), d(ctx.minecraft_jar)),
            (
                "MINECRAFT_VERSION".to_owned(),
                ctx.minecraft_version.to_owned(),
            ),
            ("ROOT".to_owned(), d(ctx.root)),
            ("INSTALLER".to_owned(), d(ctx.installer_jar)),
            ("LIBRARY_DIR".to_owned(), d(ctx.libraries_dir)),
        ]);
        for (k, v) in &self.data {
            let value = &v.client;
            let resolved = if let Some(coord) =
                value.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            {
                d(&maven_path(ctx.libraries_dir, coord)?)
            } else if let Some(lit) = value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                lit.to_owned()
            } else if let Some(rel) = value.strip_prefix('/') {
                validate_relative(rel)?;
                d(&rel
                    .split('/')
                    .fold(ctx.extracted_dir.to_path_buf(), |p, s| p.join(s)))
            } else {
                value.clone()
            };
            map.insert(k.clone(), resolved);
        }
        Ok(map)
    }
}

/// Absolute path of a Maven coordinate inside the libraries directory.
pub fn maven_path(libraries_dir: &Path, coord: &str) -> Result<PathBuf, CoreError> {
    let c = MavenCoord::parse(coord)?;
    let rel = c.path();
    validate_relative(&rel)?;
    Ok(rel
        .split('/')
        .fold(libraries_dir.to_path_buf(), |p, s| p.join(s)))
}

/// Resolve one processor argument: `{KEY}` from data, `[coord]` to a
/// library path, anything else verbatim. Unknown `{KEY}` is an error so a
/// processor never receives a literal placeholder.
pub fn resolve_arg(
    arg: &str,
    data: &HashMap<String, String>,
    libraries_dir: &Path,
) -> Result<String, CoreError> {
    if let Some(key) = arg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return data
            .get(key)
            .cloned()
            .ok_or_else(|| CoreError::MissingField {
                version: "install_profile".into(),
                field: "processor data key",
            });
    }
    if let Some(coord) = arg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(maven_path(libraries_dir, coord)?.display().to_string());
    }
    Ok(arg.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"{
      "spec": 1, "minecraft": "1.21.1", "json": "/version.json",
      "data": {
        "MAPPINGS": {"client": "[net.neoforged:neoform:1.21.1-20240808.144430:mappings@txt]", "server": ""},
        "BINPATCH": {"client": "/data/client.lzma", "server": "/data/server.lzma"},
        "MCP_VERSION": {"client": "'1.21.1-20240808.144430'", "server": ""},
        "PATCHED": {"client": "[net.neoforged:neoforge:21.1.265:client]", "server": ""}
      },
      "processors": [
        {"sides": ["server"], "jar": "a:b:1", "args": ["--task", "EXTRACT_FILES"]},
        {"jar": "net.neoforged.installertools:installertools:2.1.2", "classpath": ["x:y:1"],
         "args": ["--task", "MCP_DATA", "--input", "[net.neoforged:neoform:1.21.1-20240808.144430@zip]", "--output", "{MAPPINGS}", "--side", "{SIDE}"]},
        {"sides": ["client"], "jar": "net.neoforged.installertools:binarypatcher:2.1.2:fatjar",
         "args": ["--clean", "{MINECRAFT_JAR}", "--output", "{PATCHED}", "--apply", "{BINPATCH}"]}
      ],
      "libraries": []
    }"#;

    fn ctx<'a>(libs: &'a Path, ex: &'a Path, mc: &'a Path) -> ResolveContext<'a> {
        ResolveContext {
            libraries_dir: libs,
            extracted_dir: ex,
            minecraft_jar: mc,
            installer_jar: mc,
            root: libs,
            minecraft_version: "1.21.1",
        }
    }

    #[test]
    fn parses_and_resolves_client_processors() {
        let p: InstallProfile = serde_json::from_str(PROFILE).unwrap();
        p.check_supported().unwrap();
        let client: Vec<_> = p.processors.iter().filter(|x| x.runs_on_client()).collect();
        assert_eq!(client.len(), 2);
        assert_eq!(client[0].task(), Some("MCP_DATA"));
        assert_eq!(p.embedded_files().unwrap(), ["data/client.lzma"]);

        let (libs, ex, mc) = (
            Path::new(r"C:\NC\libraries"),
            Path::new(r"C:\NC\tmp\x"),
            Path::new(r"C:\NC\versions\v\v.jar"),
        );
        let data = p.client_data(&ctx(libs, ex, mc)).unwrap();
        assert_eq!(data["MCP_VERSION"], "1.21.1-20240808.144430");
        assert_eq!(data["BINPATCH"], r"C:\NC\tmp\x\data\client.lzma");
        assert_eq!(data["SIDE"], "client");
        let args: Vec<String> = client[1]
            .args
            .iter()
            .map(|a| resolve_arg(a, &data, libs).unwrap())
            .collect();
        assert_eq!(args[1], r"C:\NC\versions\v\v.jar");
        assert_eq!(
            args[3],
            r"C:\NC\libraries\net\neoforged\neoforge\21.1.265\neoforge-21.1.265-client.jar"
        );
        let mcp: Vec<String> = client[0]
            .args
            .iter()
            .map(|a| resolve_arg(a, &data, libs).unwrap())
            .collect();
        assert!(
            mcp[3].ends_with(r"neoform\1.21.1-20240808.144430\neoform-1.21.1-20240808.144430.zip")
        );
        assert_eq!(mcp[7], "client");
    }

    #[test]
    fn rejects_unknown_keys_traversal_and_unsupported_spec() {
        let libs = Path::new(r"C:\NC\libraries");
        assert!(resolve_arg("{NOPE}", &HashMap::new(), libs).is_err());
        assert!(resolve_arg("[a:b:../../x]", &HashMap::new(), libs).is_err());
        let mut p: InstallProfile = serde_json::from_str(PROFILE).unwrap();
        p.data.insert(
            "EVIL".into(),
            SidedValue {
                client: "/../../windows/x".into(),
                server: String::new(),
            },
        );
        assert!(p.embedded_files().is_err());
        assert!(p.client_data(&ctx(libs, libs, libs)).is_err());
        p.spec = 2;
        assert!(p.check_supported().is_err());
    }
}
