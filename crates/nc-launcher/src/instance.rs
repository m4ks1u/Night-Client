//! Instance folders and launch preparation.

use std::path::PathBuf;

use nc_core::{
    launch::{self, LaunchAuth, LaunchCommand, LaunchOptions, LaunchPaths},
    rules::Environment,
};

use crate::{LauncherError, install, layout::Layout, meta::read_json};

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
}

/// Build the launch command for an installed version in an instance folder.
/// Extracts legacy natives as needed. Does not spawn anything.
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
    let natives = layout.natives_dir(&req.dir_name, id)?;
    install::prepare_natives(layout.libraries_dir(), libs.clone(), natives.clone()).await?;

    let paths = LaunchPaths {
        java: layout
            .runtime_dir(&java_req.component)?
            .join("bin")
            .join("javaw.exe"),
        game_dir: ensure_game_dir(layout, &req.dir_name)?,
        assets_root: layout.assets_dir(),
        libraries_dir: layout.libraries_dir(),
        natives_dir: natives,
        client_jar: layout.client_jar(id)?,
        log_config: version
            .logging
            .as_ref()
            .and_then(|l| l.client.as_ref())
            .and_then(|c| layout.log_config(&c.file.id).ok()),
    };
    if !paths.java.is_file() {
        return Err(LauncherError::RuntimeUnavailable(format!(
            "{} is not installed; repair the version",
            java_req.component
        )));
    }
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
