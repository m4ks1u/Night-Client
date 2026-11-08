//! Modrinth modpacks (`.mrpack`): the `modrinth.index.json` model and its
//! validation. Pure: reading the zip, downloading and extracting live in
//! `nc_launcher::modpack`.
//!
//! A pack is untrusted input. Validation enforces, before anything is
//! written: format version 1 for Minecraft; a supported loader; every file
//! path relative and inside the instance folder; HTTPS downloads from the
//! hosts Modrinth allows in packs; SHA-1 and SHA-512 on every file; and caps
//! on the number of files and their total size.
//! See <https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack>.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    CoreError,
    loader::{LoaderKind, forge_supported},
    paths::validate_relative,
};

/// At most this many files per pack (large packs have a few hundred).
pub const MAX_FILES: usize = 4000;
/// At most this many bytes of downloads per pack.
pub const MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// `modrinth.index.json` larger than this is refused.
pub const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

/// Hosts a pack may download from: the ones Modrinth itself accepts.
const ALLOWED_HOSTS: [&str; 4] = [
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub files: Vec<IndexFile>,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexFile {
    pub path: String,
    pub hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub env: Option<Env>,
    pub downloads: Vec<String>,
    pub file_size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Env {
    #[serde(default)]
    pub client: Option<String>,
}

/// A file to download into the instance folder, validated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    /// Relative path inside the instance folder (`mods/x.jar`).
    pub path: String,
    pub url: String,
    pub sha1: String,
    pub sha512: String,
    pub size: u64,
    /// The pack marks it optional on the client; installed by default.
    pub optional: bool,
}

/// A validated pack, ready to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub minecraft: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    pub files: Vec<PackFile>,
    /// Files the pack marks unsupported on the client (server-only).
    pub skipped: usize,
}

impl Pack {
    pub fn download_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

fn bad(field: &'static str, what: impl Into<String>) -> CoreError {
    CoreError::MissingField {
        version: what.into(),
        field,
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Allowed download URL: HTTPS on an allowed host, no credentials or port.
pub fn allowed_url(u: &str) -> bool {
    url::Url::parse(u).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
            && u.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
    })
}

/// Loader from the pack's `dependencies` (exactly one, or none for vanilla).
fn loader_of(deps: &BTreeMap<String, String>) -> Result<(LoaderKind, Option<String>), CoreError> {
    let mut found = Vec::new();
    for (k, v) in deps {
        let kind = match k.as_str() {
            "minecraft" => continue,
            "fabric-loader" => LoaderKind::Fabric,
            "quilt-loader" => LoaderKind::Quilt,
            "neoforge" => LoaderKind::NeoForge,
            "forge" => LoaderKind::Forge,
            other => {
                return Err(bad(
                    "supported loader",
                    format!("modpack dependency {other}"),
                ));
            }
        };
        found.push((kind, Some(v.clone())));
    }
    match found.len() {
        0 => Ok((LoaderKind::Vanilla, None)),
        1 => Ok(found.remove(0)),
        _ => Err(bad("a single loader", "modpack dependencies")),
    }
}

impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(bytes).map_err(|e| bad("valid modrinth.index.json", e.to_string()))
    }

    /// Validate everything and keep the files the client needs.
    pub fn validate(self) -> Result<Pack, CoreError> {
        if self.format_version != 1 || self.game != "minecraft" {
            return Err(bad(
                "formatVersion 1 for Minecraft",
                format!("modpack format {} for {}", self.format_version, self.game),
            ));
        }
        let name = self.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
            return Err(bad("name", "modpack"));
        }
        let minecraft = self
            .dependencies
            .get("minecraft")
            .cloned()
            .ok_or_else(|| bad("minecraft dependency", "modpack"))?;
        if minecraft.is_empty()
            || minecraft.len() > 64
            || !minecraft
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(bad("valid Minecraft version", minecraft));
        }
        let (loader, loader_version) = loader_of(&self.dependencies)?;
        if loader == LoaderKind::Forge && !forge_supported(&minecraft) {
            return Err(bad(
                "supported Forge version",
                format!("Forge for {minecraft}"),
            ));
        }
        if let Some(v) = &loader_version
            && (v.is_empty()
                || v.len() > 64
                || !v
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_')))
        {
            return Err(bad("valid loader version", v.clone()));
        }
        if self.files.len() > MAX_FILES {
            return Err(bad(
                "at most 4000 files",
                format!("{} files", self.files.len()),
            ));
        }

        let mut files = Vec::new();
        let mut skipped = 0;
        let mut seen = HashSet::new();
        let mut total = 0u64;
        for f in self.files {
            let client = f.env.as_ref().and_then(|e| e.client.as_deref());
            if client == Some("unsupported") {
                skipped += 1;
                continue;
            }
            if !instance_path_allowed(&f.path) {
                return Err(bad("a path inside the instance folder", f.path));
            }
            if !seen.insert(f.path.to_ascii_lowercase()) {
                return Err(bad("unique file paths", f.path));
            }
            let sha1 = f.hashes.get("sha1").map(|s| s.to_ascii_lowercase());
            let sha512 = f.hashes.get("sha512").map(|s| s.to_ascii_lowercase());
            let (Some(sha1), Some(sha512)) = (sha1, sha512) else {
                return Err(bad("sha1 and sha512 hashes", f.path));
            };
            if !is_hex(&sha1, 40) || !is_hex(&sha512, 128) {
                return Err(bad("sha1 and sha512 hashes", f.path));
            }
            let url = f
                .downloads
                .iter()
                .find(|u| allowed_url(u))
                .cloned()
                .ok_or_else(|| bad("download from an allowed host", f.path.clone()))?;
            total = total.saturating_add(f.file_size);
            files.push(PackFile {
                path: f.path,
                url,
                sha1,
                sha512,
                size: f.file_size,
                optional: client == Some("optional"),
            });
        }
        if total > MAX_DOWNLOAD_BYTES {
            return Err(bad("at most 8 GiB of downloads", format!("{total} bytes")));
        }
        Ok(Pack {
            name,
            version_id: self.version_id.chars().take(64).collect(),
            summary: self.summary.map(|s| s.chars().take(500).collect()),
            minecraft,
            loader,
            loader_version,
            files,
            skipped,
        })
    }
}

/// Top-level folders of an instance that belong to the launcher:
/// `.natives` (native libraries extracted for the game to load) and
/// `.nc-update` (modpack update staging and backups).
pub const RESERVED_DIRS: [&str; 2] = [".natives", ".nc-update"];

/// True for a launcher-owned top-level folder name, in any letter case.
pub fn is_reserved_dir(name: &str) -> bool {
    RESERVED_DIRS.iter().any(|r| name.eq_ignore_ascii_case(r))
}

/// A path a pack may write inside the instance folder: a safe relative
/// path that is not in one of the [`RESERVED_DIRS`].
pub fn instance_path_allowed(path: &str) -> bool {
    validate_relative(path).is_ok() && !path.split('/').next().is_some_and(is_reserved_dir)
}

/// Map an override entry (`overrides/config/x.toml`) to its path inside the
/// instance. `client-overrides/` entries win over `overrides/` ones; other
/// entries (`modrinth.index.json`) map to nothing. Returns (path, priority).
pub fn override_target(entry: &str) -> Option<(&str, u8)> {
    if let Some(rest) = entry.strip_prefix("client-overrides/") {
        return (!rest.is_empty()).then_some((rest, 1));
    }
    entry
        .strip_prefix("overrides/")
        .filter(|r| !r.is_empty())
        .map(|r| (r, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    fn index(files: serde_json::Value, deps: serde_json::Value) -> Index {
        Index::parse(
            serde_json::json!({
                "formatVersion": 1, "game": "minecraft", "versionId": "1.2.0",
                "name": "Example Pack", "summary": "A pack", "files": files, "dependencies": deps
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    fn file(path: &str, url: &str, client: &str) -> serde_json::Value {
        serde_json::json!({
            "path": path, "hashes": {"sha1": h('a', 40), "sha512": h('B', 128)},
            "env": {"client": client, "server": "required"},
            "downloads": [url], "fileSize": 100
        })
    }

    const CDN: &str = "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar";

    #[test]
    fn validates_a_fabric_pack_and_skips_server_only_files() {
        let p = index(
            serde_json::json!([
                file("mods/sodium.jar", CDN, "required"),
                file("mods/server-only.jar", CDN, "unsupported"),
                file(
                    "resourcepacks/extra.zip",
                    "https://github.com/a/b/releases/download/1/extra.zip",
                    "optional"
                ),
            ]),
            serde_json::json!({"minecraft": "1.21.1", "fabric-loader": "0.16.10"}),
        )
        .validate()
        .unwrap();
        assert_eq!(
            (p.loader, p.loader_version.as_deref()),
            (LoaderKind::Fabric, Some("0.16.10"))
        );
        assert_eq!(p.minecraft, "1.21.1");
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.skipped, 1);
        assert!(p.files[1].optional && !p.files[0].optional);
        assert_eq!(p.files[0].sha512, h('b', 128), "hashes are lowercased");
        assert_eq!(p.download_bytes(), 200);
    }

    #[test]
    fn maps_every_loader_and_vanilla() {
        for (dep, kind) in [
            ("quilt-loader", LoaderKind::Quilt),
            ("neoforge", LoaderKind::NeoForge),
            ("forge", LoaderKind::Forge),
        ] {
            let p = index(
                serde_json::json!([]),
                serde_json::json!({"minecraft": "1.20.1", dep: "1.0"}),
            )
            .validate()
            .unwrap();
            assert_eq!(p.loader, kind);
        }
        let v = index(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "1.21.1"}),
        )
        .validate()
        .unwrap();
        assert_eq!((v.loader, v.loader_version), (LoaderKind::Vanilla, None));
    }

    #[test]
    fn refuses_unsafe_or_unsupported_packs() {
        let fabric = serde_json::json!({"minecraft": "1.21.1", "fabric-loader": "0.16.10"});
        let refuse = |files: serde_json::Value, deps: serde_json::Value| {
            index(files, deps).validate().is_err()
        };
        for path in [
            ".natives/evil.dll",
            "../evil.jar",
            "/abs.jar",
            r"mods\x.jar",
            "mods/../../x.jar",
            "C:/x.jar",
            "mods/con.jar",
        ] {
            assert!(
                refuse(
                    serde_json::json!([file(path, CDN, "required")]),
                    fabric.clone()
                ),
                "{path}"
            );
        }
        for url in [
            "http://cdn.modrinth.com/x.jar",
            "https://evil.example/x.jar",
            "https://cdn.modrinth.com.evil.example/x.jar",
            "https://user:pw@cdn.modrinth.com/x.jar",
            "https://cdn.modrinth.com:8443/x.jar",
        ] {
            assert!(
                refuse(
                    serde_json::json!([file("mods/x.jar", url, "required")]),
                    fabric.clone()
                ),
                "{url}"
            );
        }
        // Duplicate paths (case-insensitively, as on Windows).
        assert!(refuse(
            serde_json::json!([
                file("mods/a.jar", CDN, "required"),
                file("mods/A.jar", CDN, "required")
            ]),
            fabric.clone()
        ));
        // Missing or malformed hashes.
        let mut nohash = file("mods/x.jar", CDN, "required");
        nohash["hashes"] = serde_json::json!({"sha1": h('a', 40)});
        assert!(refuse(serde_json::json!([nohash]), fabric.clone()));
        let mut shorthash = file("mods/x.jar", CDN, "required");
        shorthash["hashes"]["sha512"] = serde_json::json!("abc");
        assert!(refuse(serde_json::json!([shorthash]), fabric.clone()));
        // Loaders: unknown, two at once, Forge before 1.12.2, odd versions.
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "1.21.1", "liteloader": "1"})
        ));
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "1.21.1", "forge": "1", "neoforge": "2"})
        ));
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "1.7.10", "forge": "10.13.4.1614"})
        ));
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "1.21.1", "fabric-loader": "0.16 && x"})
        ));
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"minecraft": "../1.21"})
        ));
        assert!(refuse(
            serde_json::json!([]),
            serde_json::json!({"fabric-loader": "0.16.10"})
        ));
        // Format.
        let mut wrong = index(serde_json::json!([]), fabric.clone());
        wrong.format_version = 2;
        assert!(wrong.validate().is_err());
        // Size cap.
        let mut huge = file("mods/x.jar", CDN, "required");
        huge["fileSize"] = serde_json::json!(MAX_DOWNLOAD_BYTES + 1);
        assert!(refuse(serde_json::json!([huge]), fabric));
    }

    #[test]
    fn override_entries() {
        assert_eq!(
            override_target("overrides/config/a.toml"),
            Some(("config/a.toml", 0))
        );
        assert_eq!(
            override_target("client-overrides/options.txt"),
            Some(("options.txt", 1))
        );
        assert_eq!(override_target("overrides/"), None);
        assert_eq!(override_target("modrinth.index.json"), None);
        assert_eq!(override_target("server-overrides/x"), None);
        assert!(instance_path_allowed("config/a.toml"));
        assert!(
            !instance_path_allowed(".natives/lwjgl.dll") && !instance_path_allowed(".NATIVES/x")
        );
        assert!(!instance_path_allowed("../x") && !instance_path_allowed(""));
        for staging in [".nc-update/x", ".NC-Update/journal.json", ".nc-update"] {
            assert!(!instance_path_allowed(staging), "{staging}");
        }
        // Windows ignores a trailing dot or space; those are refused too.
        assert!(!instance_path_allowed(".nc-update./x") && !instance_path_allowed(".nc-update /x"));
        assert!(
            instance_path_allowed("config/.nc-update/x") && instance_path_allowed(".nc-updates/x")
        );
    }
}
