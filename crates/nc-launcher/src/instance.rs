//! Instance folders and launch preparation.

use std::path::{Path, PathBuf};

use nc_core::{
    java::{Compatibility, PeArch, compatibility},
    launch::{self, LaunchAuth, LaunchCommand, LaunchOptions, LaunchPaths},
    manifest::AssetIndex,
    rules::Environment,
};

use crate::{
    LauncherError, fsutil, install,
    javadetect::{self, JavaSource},
    layout::Layout,
    meta::read_json,
};

/// `<data>/instances/<dir_name>`: the instance's game directory.
pub fn game_dir(layout: &Layout, dir_name: &str) -> Result<PathBuf, LauncherError> {
    Ok(layout.instance_dir(dir_name)?)
}

pub fn ensure_game_dir(layout: &Layout, dir_name: &str) -> Result<PathBuf, LauncherError> {
    let dir = game_dir(layout, dir_name)?;
    std::fs::create_dir_all(&dir).map_err(LauncherError::io(&dir))?;
    Ok(dir)
}

/// Move an instance folder to `<data>/trash/<dir>-<unix time>` instead of
/// deleting it, so a mistaken delete can be recovered by hand.
/// Returns the trash location, or `None` if the folder did not exist.
pub fn move_to_trash(layout: &Layout, dir_name: &str) -> Result<Option<PathBuf>, LauncherError> {
    let src = game_dir(layout, dir_name)?;
    if !src.exists() {
        return Ok(None);
    }
    let trash = layout.root().join("trash");
    std::fs::create_dir_all(&trash).map_err(LauncherError::io(&trash))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = trash.join(format!("{dir_name}-{stamp}"));
    std::fs::rename(&src, &dest).map_err(LauncherError::io(&src))?;
    Ok(Some(dest))
}

pub struct LaunchRequest {
    pub version_id: String,
    /// Instance folder name (also used for the natives directory).
    pub dir_name: String,
    pub auth: LaunchAuth,
    pub max_memory_mb: u32,
    pub extra_jvm_args: Vec<String>,
    /// User-chosen Java; `None` uses the official Mojang runtime.
    pub java_override: Option<PathBuf>,
}

/// Check a Java runtime against what a version needs. Returns the `javaw.exe`
/// to start, or an actionable error.
pub fn check_java(
    path: &Path,
    required_major: u32,
) -> Result<(PathBuf, Compatibility), LauncherError> {
    let rt = javadetect::inspect(path, JavaSource::Custom)
        .map_err(|e| LauncherError::JavaIncompatible(e.to_string()))?;
    let compat = compatibility(required_major, rt.major, rt.arch == PeArch::X64);
    if compat == Compatibility::Incompatible {
        return Err(LauncherError::JavaIncompatible(if rt.arch != PeArch::X64 {
            format!("{} is not a 64-bit Java runtime", rt.home.display())
        } else {
            format!(
                "this version needs Java {required_major} or newer, but {} is Java {}",
                rt.home.display(),
                rt.major
            )
        }));
    }
    Ok((rt.path, compat))
}

/// Build the launch command for an installed version in an instance folder.
/// Verifies required files exist, extracts legacy natives and lays out
/// legacy assets. Does not spawn anything.
pub async fn prepare_launch(
    layout: &Layout,
    req: LaunchRequest,
) -> Result<LaunchCommand, LauncherError> {
    let id = &req.version_id;
    if !layout.installed_marker(id)?.is_file() {
        return Err(LauncherError::NotInstalled(id.clone()));
    }
    let version: nc_core::manifest::VersionJson =
        read_json(&layout.version_json(id)?, "version").await?;
    let env = Environment::windows_x64();
    let libs = nc_core::libraries::resolve(&version, &env)?;
    let java_req = nc_core::java::requirement(&version);
    let game_dir = ensure_game_dir(layout, &req.dir_name)?;

    let java = match &req.java_override {
        Some(p) => {
            let (exe, compat) = check_java(p, java_req.major_version)?;
            if compat == Compatibility::Risky {
                tracing::warn!(version = %id, java = %exe.display(), "launching an old version with a newer Java than it expects");
            }
            exe
        }
        None => {
            let exe = layout
                .runtime_dir(&java_req.component)?
                .join("bin")
                .join("javaw.exe");
            if !exe.is_file() {
                return Err(LauncherError::FilesMissing(format!(
                    "Java runtime {}",
                    java_req.component
                )));
            }
            exe
        }
    };

    // Cheap existence check so a damaged install fails with "verify files"
    // instead of an obscure ClassNotFoundException inside the game.
    let client_jar = layout.client_jar(id)?;
    let mut required = vec![client_jar.clone()];
    for l in &libs {
        required.push(fsutil::safe_join(&layout.libraries_dir(), &l.path)?);
    }
    if let Some(missing) = required.iter().find(|p| !p.is_file()) {
        return Err(LauncherError::FilesMissing(
            missing
                .strip_prefix(layout.root())
                .unwrap_or(missing)
                .display()
                .to_string(),
        ));
    }

    let natives = layout.natives_dir(&req.dir_name, id)?;
    install::prepare_natives(layout.libraries_dir(), libs.clone(), natives.clone()).await?;

    // Legacy asset layouts.
    let mut game_assets = layout.assets_dir();
    if let Some(ai) = &version.asset_index {
        let index: AssetIndex = read_json(&layout.asset_index(&ai.id)?, "asset index").await?;
        if index.map_to_resources {
            let dest = game_dir.join("resources");
            let root = layout.assets_dir();
            tokio::task::spawn_blocking({
                let dest = dest.clone();
                move || install::materialize_assets(&index, &root, &dest)
            })
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))??;
            game_assets = dest;
        } else if index.is_virtual {
            game_assets = layout.assets_dir().join("virtual").join(&ai.id);
        }
    }

    let paths = LaunchPaths {
        java,
        game_dir,
        assets_root: layout.assets_dir(),
        libraries_dir: layout.libraries_dir(),
        natives_dir: natives,
        client_jar,
        game_assets,
        log_config: version
            .logging
            .as_ref()
            .and_then(|l| l.client.as_ref())
            .and_then(|c| layout.log_config(&c.file.id).ok())
            .filter(|p| p.is_file()),
    };
    let opts = LaunchOptions {
        min_memory_mb: None,
        max_memory_mb: req.max_memory_mb,
        extra_jvm_args: req.extra_jvm_args,
        resolution: None,
        launcher_name: "night-client".into(),
        launcher_version: env!("CARGO_PKG_VERSION").into(),
    };
    Ok(launch::build(
        &version, &env, &libs, &paths, &req.auth, &opts,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_moves_folder_and_rejects_traversal() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::new(root.path());
        let dir = ensure_game_dir(&layout, "World").unwrap();
        std::fs::write(dir.join("options.txt"), "x").unwrap();
        let moved = move_to_trash(&layout, "World").unwrap().unwrap();
        assert!(!dir.exists());
        assert!(moved.join("options.txt").is_file());
        assert!(moved.starts_with(root.path().join("trash")));
        assert_eq!(move_to_trash(&layout, "Missing").unwrap(), None);
        assert!(move_to_trash(&layout, "..").is_err());
        assert!(move_to_trash(&layout, "../x").is_err());
    }
}
