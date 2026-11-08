//! Kinds of Modrinth content an instance can hold, and where each lives.
//!
//! Mods go in `mods/` and need a mod loader. Resource packs (`resourcepacks/`)
//! work on every instance, vanilla included. Shader packs (`shaderpacks/`) are
//! read by a shader mod (Iris, or OptiFine); Modrinth tags "vanilla" packs that
//! use the game's own core shaders.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    #[default]
    Mod,
    ResourcePack,
    Shader,
}

impl ContentKind {
    pub const ALL: [ContentKind; 3] = [Self::Mod, Self::ResourcePack, Self::Shader];

    /// Modrinth `project_type`, also the stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::ResourcePack => "resourcepack",
            Self::Shader => "shader",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Folder inside the game directory.
    pub fn folder(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::ResourcePack => "resourcepacks",
            Self::Shader => "shaderpacks",
        }
    }

    /// File extension (lowercase, with the dot) of an installable file.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mod => ".jar",
            Self::ResourcePack | Self::Shader => ".zip",
        }
    }

    /// Whether an instance with `loader` can use this kind at all.
    pub fn available_for(self, loader: &str) -> bool {
        match self {
            Self::Mod => !crate::loader::mod_loaders(loader).is_empty(),
            Self::ResourcePack | Self::Shader => true,
        }
    }

    /// Modrinth loader tags accepted on an instance with `loader`, the
    /// instance's own first (it is preferred when builds tie).
    pub fn modrinth_loaders(self, loader: &str) -> &'static [&'static str] {
        match self {
            Self::Mod => crate::loader::mod_loaders(loader),
            Self::ResourcePack => &["minecraft"],
            // Core-shader packs work everywhere; Iris and OptiFine packs need
            // a shader mod, which only a modded instance can have.
            Self::Shader if loader == "vanilla" => &["vanilla"],
            Self::Shader => &["iris", "optifine", "vanilla"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ContentKind::*;
    use super::*;

    #[test]
    fn folders_extensions_and_loaders() {
        assert_eq!(
            ContentKind::ALL.map(ContentKind::folder),
            ["mods", "resourcepacks", "shaderpacks"]
        );
        assert_eq!(ResourcePack.extension(), ".zip");
        assert_eq!(ContentKind::parse("shader"), Some(Shader));
        assert_eq!(ContentKind::parse("datapack"), None);
        assert_eq!(Mod.modrinth_loaders("quilt"), ["quilt", "fabric"]);
        assert_eq!(ResourcePack.modrinth_loaders("vanilla"), ["minecraft"]);
        assert_eq!(Shader.modrinth_loaders("vanilla"), ["vanilla"]);
        assert_eq!(Shader.modrinth_loaders("fabric")[0], "iris");
        assert!(!Mod.available_for("vanilla") && Mod.available_for("forge"));
        assert!(ResourcePack.available_for("vanilla") && Shader.available_for("vanilla"));
        assert_eq!(
            serde_json::to_string(&ResourcePack).unwrap(),
            "\"resourcepack\""
        );
    }
}
