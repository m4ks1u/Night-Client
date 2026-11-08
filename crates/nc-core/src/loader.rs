//! Loader identity: which game version + loader + loader version an
//! instance runs, and the version id it launches.

use serde::{Deserialize, Serialize};

use crate::{CoreError, paths::validate_segment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    #[serde(rename = "neoforge")]
    NeoForge,
    Forge,
}

impl LoaderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::NeoForge => "neoforge",
            Self::Forge => "forge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vanilla" => Some(Self::Vanilla),
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "neoforge" => Some(Self::NeoForge),
            "forge" => Some(Self::Forge),
            _ => None,
        }
    }
}

/// Modrinth loader names whose mods run on `loader` (a Modrinth/loader name
/// such as `quilt`), the instance's own first. Quilt runs Fabric mods.
pub fn mod_loaders(loader: &str) -> &'static [&'static str] {
    match loader {
        "fabric" => &["fabric"],
        "quilt" => &["quilt", "fabric"],
        "neoforge" => &["neoforge"],
        "forge" => &["forge"],
        _ => &[],
    }
}

/// What to install: `vanilla:<mc>`, `fabric:<mc>:<loader>`, `quilt:<mc>:<loader>`, `neoforge:<mc>:<neoforge>`,
/// `forge:<mc>:<forge>` (the Forge version without the `<mc>-` prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstallKey {
    pub kind: LoaderKind,
    pub minecraft: String,
    pub loader_version: Option<String>,
}

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | ' '))
        && validate_segment(s).is_ok()
}

impl InstallKey {
    pub fn new(
        kind: LoaderKind,
        minecraft: &str,
        loader_version: Option<&str>,
    ) -> Result<Self, CoreError> {
        let bad = || {
            CoreError::UnsafePath(format!(
                "{}:{minecraft}:{}",
                kind.as_str(),
                loader_version.unwrap_or("")
            ))
        };
        if !valid_part(minecraft) {
            return Err(bad());
        }
        let loader_version = match (kind, loader_version) {
            (LoaderKind::Vanilla, _) => None,
            (_, Some(v)) if valid_part(v) => Some(v.to_owned()),
            _ => return Err(bad()),
        };
        Ok(Self {
            kind,
            minecraft: minecraft.to_owned(),
            loader_version,
        })
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let mut parts = s.splitn(3, ':');
        let kind = parts
            .next()
            .and_then(LoaderKind::parse)
            .ok_or_else(|| CoreError::UnsafePath(s.to_owned()))?;
        let mc = parts
            .next()
            .ok_or_else(|| CoreError::UnsafePath(s.to_owned()))?;
        let lv = parts.next();
        if kind == LoaderKind::Vanilla && lv.is_some() {
            return Err(CoreError::UnsafePath(s.to_owned()));
        }
        Self::new(kind, mc, lv)
    }

    pub fn to_key(&self) -> String {
        match &self.loader_version {
            Some(v) => format!("{}:{}:{}", self.kind.as_str(), self.minecraft, v),
            None => format!("{}:{}", self.kind.as_str(), self.minecraft),
        }
    }

    /// The version id this install is launched as (`versions/<id>/<id>.json`).
    pub fn launch_id(&self) -> String {
        match (&self.kind, &self.loader_version) {
            (LoaderKind::Fabric, Some(v)) => format!("fabric-loader-{v}-{}", self.minecraft),
            (LoaderKind::Quilt, Some(v)) => format!("quilt-loader-{v}-{}", self.minecraft),
            (LoaderKind::NeoForge, Some(v)) => format!("neoforge-{v}"),
            (LoaderKind::Forge, Some(v)) => format!("{}-forge-{v}", self.minecraft),
            _ => self.minecraft.clone(),
        }
    }

    /// Short human label, e.g. "Fabric 0.17.2 · 1.21.1".
    pub fn label(&self) -> String {
        match (&self.kind, &self.loader_version) {
            (LoaderKind::Fabric, Some(v)) => format!("Fabric {v} · {}", self.minecraft),
            (LoaderKind::Quilt, Some(v)) => format!("Quilt {v} · {}", self.minecraft),
            (LoaderKind::NeoForge, Some(v)) => format!("NeoForge {v} · {}", self.minecraft),
            (LoaderKind::Forge, Some(v)) => format!("Forge {v} · {}", self.minecraft),
            _ => format!("Minecraft {}", self.minecraft),
        }
    }
}

/// NeoForge version prefix for a Minecraft version:
/// `1.21.1` → `21.1.`, `1.21` → `21.0.`, `26.3` → `26.3.0.`, `26.1.2` → `26.1.2.`.
pub fn neoforge_prefix(minecraft: &str) -> Option<String> {
    let parts: Vec<&str> = minecraft.split('.').collect();
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    match parts.as_slice() {
        ["1", minor] => Some(format!("{minor}.0.")),
        ["1", minor, patch] => Some(format!("{minor}.{patch}.")),
        [year, drop] if year.len() == 2 && *year != "1" => Some(format!("{year}.{drop}.0.")),
        [year, drop, hotfix] if year.len() == 2 => Some(format!("{year}.{drop}.{hotfix}.")),
        _ => None,
    }
}

/// NeoForge versions for `minecraft`, newest first; `-beta` builds flagged.
pub fn neoforge_versions_for<'a>(
    minecraft: &str,
    all: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, bool)> {
    let Some(prefix) = neoforge_prefix(minecraft) else {
        return Vec::new();
    };
    let mut v: Vec<(String, bool)> = all
        .into_iter()
        .filter(|v| {
            v.starts_with(&prefix)
                && v[prefix.len()..]
                    .split(['.', '-'])
                    .next()
                    .is_some_and(|b| !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|v| (v.to_owned(), !v.contains("beta") && !v.contains("alpha")))
        .collect();
    v.sort_by(|a, b| version_cmp(&b.0, &a.0));
    v
}

/// Forge's installer uses the supported `install_profile` spec 1 from
/// Minecraft 1.17 on; older Forge installers are spec 0.
pub fn forge_supported(minecraft: &str) -> bool {
    let mut parts = minecraft.split('.');
    match (parts.next(), parts.next()) {
        (Some("1"), Some(minor)) => minor.parse::<u32>().is_ok_and(|m| m >= 17),
        // Year-based versions (26.x) are all newer.
        (Some(year), Some(_)) => year.len() == 2 && year.parse::<u32>().is_ok_and(|y| y >= 25),
        _ => false,
    }
}

/// Forge versions for `minecraft` from Maven's `<mc>-<forge>` list, without
/// the prefix, newest first. Empty for versions whose installer is not spec 1.
pub fn forge_versions_for<'a>(
    minecraft: &str,
    all: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    if !forge_supported(minecraft) {
        return Vec::new();
    }
    let prefix = format!("{minecraft}-");
    let mut v: Vec<String> = all
        .into_iter()
        .filter_map(|v| v.strip_prefix(&prefix))
        .filter(|rest| {
            !rest.is_empty()
                && rest
                    .split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
        .collect();
    v.sort_by(|a, b| version_cmp(b, a));
    v
}

/// Compare dotted numeric versions (non-numeric parts compare as text).
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        s.split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().map_err(|_| p.to_owned()))
            .collect::<Vec<_>>()
    };
    let (a, b) = (split(a), split(b));
    for (x, y) in a.iter().zip(b.iter()) {
        let o = match (x, y) {
            (Ok(x), Ok(y)) => x.cmp(y),
            (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
            (Err(_), Ok(_)) => std::cmp::Ordering::Less,
            (Err(x), Err(y)) => x.cmp(y),
        };
        if o.is_ne() {
            return o;
        }
    }
    // Equal prefix: a numeric extra part is a newer build (1.0 < 1.0.1),
    // a text extra part is a pre-release (1.0-beta < 1.0).
    match (a.get(b.len()), b.get(a.len())) {
        (Some(Ok(_)), _) => std::cmp::Ordering::Greater,
        (Some(Err(_)), _) => std::cmp::Ordering::Less,
        (_, Some(Ok(_))) => std::cmp::Ordering::Less,
        (_, Some(Err(_))) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_launch_ids() {
        for (k, id) in [
            ("vanilla:1.21.1", "1.21.1"),
            ("fabric:1.21.1:0.17.2", "fabric-loader-0.17.2-1.21.1"),
            ("neoforge:1.21.1:21.1.265", "neoforge-21.1.265"),
            ("quilt:1.21.1:0.29.2", "quilt-loader-0.29.2-1.21.1"),
            ("forge:1.20.1:47.4.29", "1.20.1-forge-47.4.29"),
            (
                "fabric:1.12.2:0.14.0+build.1",
                "fabric-loader-0.14.0+build.1-1.12.2",
            ),
        ] {
            let key = InstallKey::parse(k).unwrap();
            assert_eq!(key.to_key(), k);
            assert_eq!(key.launch_id(), id);
        }
        for bad in [
            "",
            "vanilla:1.21.1:x",
            "fabric:1.21.1",
            "quilt:1.21.1",
            "fabric:../x:0.1",
            "neoforge:1.21.1:21/..",
            "vanilla:",
        ] {
            assert!(InstallKey::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn quilt_runs_fabric_mods_but_not_the_reverse() {
        assert_eq!(mod_loaders("quilt"), ["quilt", "fabric"]);
        assert_eq!(mod_loaders("fabric"), ["fabric"]);
        assert!(mod_loaders("vanilla").is_empty() && mod_loaders("rift").is_empty());
        assert_eq!(mod_loaders("forge"), ["forge"]);
    }

    #[test]
    fn neoforge_mapping() {
        assert_eq!(neoforge_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("26.3").as_deref(), Some("26.3.0."));
        assert_eq!(neoforge_prefix("26.1.2").as_deref(), Some("26.1.2."));
        assert_eq!(neoforge_prefix("26.4-snapshot-7"), None);
        let all = [
            "21.1.9",
            "21.1.265",
            "21.1.10-beta",
            "21.10.3",
            "21.11.1",
            "26.3.0.101",
            "26.3.0.99",
            "26.1.2.126",
        ];
        assert_eq!(
            neoforge_versions_for("1.21.1", all),
            vec![
                ("21.1.265".into(), true),
                ("21.1.10-beta".into(), false),
                ("21.1.9".into(), true)
            ]
        );
        assert_eq!(
            neoforge_versions_for("26.3", all)
                .first()
                .map(|v| v.0.as_str()),
            Some("26.3.0.101")
        );
        assert!(neoforge_versions_for("1.12.2", all).is_empty());
    }

    #[test]
    fn forge_mapping() {
        let all = [
            "1.20.1-47.4.5",
            "1.20.1-47.4.29",
            "1.20.1-47.4.4",
            "1.20.10-99.0.0",
            "1.20.1-47.1.0-pre1",
            "1.21.1-52.1.21",
            "1.16.5-36.2.42",
            "1.7.10-10.13.4.1614-1.7.10",
        ];
        assert_eq!(
            forge_versions_for("1.20.1", all),
            ["47.4.29", "47.4.5", "47.4.4"],
            "sorted, exact game version, no odd suffixes"
        );
        assert!(
            forge_versions_for("1.16.5", all).is_empty(),
            "spec 0 installers"
        );
        assert!(forge_supported("1.17.1") && forge_supported("1.21") && forge_supported("26.3"));
        assert!(!forge_supported("1.16.5") && !forge_supported("1.12.2") && !forge_supported("x"));
    }

    #[test]
    fn version_ordering() {
        use std::cmp::Ordering::*;
        assert_eq!(version_cmp("0.17.2", "0.16.14"), Greater);
        assert_eq!(version_cmp("21.1.265", "21.1.99"), Greater);
        assert_eq!(
            version_cmp("21.1.10", "21.1.10-beta"),
            Greater,
            "release after its pre-release"
        );
        assert_eq!(version_cmp("1.0", "1.0.1"), Less);
        assert_eq!(version_cmp("1.0", "1.0"), Equal);
    }
}
