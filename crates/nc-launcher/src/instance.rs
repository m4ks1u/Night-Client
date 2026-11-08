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

/// Folders owned by Windows that must never hold launcher or game data.
pub(crate) fn system_dirs() -> Vec<PathBuf> {
    [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .collect()
}

/// Validate a user-chosen game directory. It must be an absolute, local or
/// UNC path that is not a drive root, not inside Windows/Program Files/
/// ProgramData, and not inside Night Client's managed folders (versions,
/// libraries, assets, runtimes, cache, trash, the vault). The folder is
/// created if missing. Returns the canonical path.
pub fn validate_custom_game_dir(layout: &Layout, input: &str) -> Result<PathBuf, LauncherError> {
    let invalid = |m: &str| LauncherError::InvalidPath(format!("{input}: {m}"));
    let p = PathBuf::from(input.trim());
    if !p.is_absolute() {
        return Err(invalid(
            "choose a full path, for example D:\\Minecraft\\Survival",
        ));
    }
    if p.parent().is_none() {
        return Err(invalid("a drive root cannot be a game folder"));
    }
    if p.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err(invalid("the path must not contain . or .. segments"));
    }
    // ':' is only allowed in the drive prefix (no alternate data streams).
    let tail = p.to_string_lossy();
    let after_prefix = tail.get(2..).unwrap_or("");
    if after_prefix.contains(':')
        || tail
            .chars()
            .any(|c| c.is_control() || matches!(c, '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(invalid(
            "the path contains characters Windows does not allow",
        ));
    }
    std::fs::create_dir_all(&p).map_err(|e| invalid(&e.to_string()))?;
    let canon = std::fs::canonicalize(&p).map_err(|e| invalid(&e.to_string()))?;
    if !canon.is_dir() {
        return Err(invalid("not a folder"));
    }

    let mut forbidden = system_dirs();
    for sub in [
        "versions",
        "libraries",
        "assets",
        "runtimes",
        "cache",
        "trash",
    ] {
        forbidden.push(layout.root().join(sub));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        forbidden.push(PathBuf::from(local).join("NightClient"));
    }
    for f in forbidden {
        let f = std::fs::canonicalize(&f).unwrap_or(f);
        if canon.starts_with(&f) {
            return Err(invalid("this folder is managed by Windows or Night Client"));
        }
    }
    if let Ok(root) = std::fs::canonicalize(layout.root())
        && canon == root
    {
        return Err(invalid(
            "choose a folder inside or outside the data folder, not the data folder itself",
        ));
    }
    Ok(canon)
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
    /// Custom game directory (already validated); `None` uses the default.
    pub game_dir: Option<PathBuf>,
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
    // Re-validate: stored arguments may have been edited outside the app.
    nc_core::jvmargs::validate(&req.extra_jvm_args)?;
    if !layout.installed_marker(id)?.is_file() {
        return Err(LauncherError::NotInstalled(id.clone()));
    }

    let version = crate::loaders::resolve_version(layout, id).await?;
    let env = Environment::windows_x64();
    let libs = nc_core::libraries::resolve(&version, &env)?;
    let java_req = nc_core::java::requirement(&version);
    let game_dir = match &req.game_dir {
        Some(d) => {
            std::fs::create_dir_all(d).map_err(LauncherError::io(d))?;
            d.clone()
        }
        None => ensure_game_dir(layout, &req.dir_name)?,
    };

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
    fn custom_game_dir_validation() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::new(root.path().join("data"));
        std::fs::create_dir_all(layout.libraries_dir()).unwrap();
        let ok = root.path().join("Games").join("Survival");
        let canon = validate_custom_game_dir(&layout, &ok.display().to_string()).unwrap();
        assert!(canon.is_dir());
        // Inside the data folder but not a managed sub-folder is fine.
        assert!(
            validate_custom_game_dir(
                &layout,
                &layout.root().join("my-worlds").display().to_string()
            )
            .is_ok()
        );
        for bad in [
            r"relative\dir".to_string(),
            r"C:\".to_string(),
            layout.libraries_dir().join("x").display().to_string(),
            layout.root().display().to_string(),
            std::env::var("SystemRoot").unwrap_or(r"C:\Windows".into()),
            format!(r"{}\x:ads", root.path().display()),
            format!(r"{}\..\escape", ok.display()),
        ] {
            assert!(validate_custom_game_dir(&layout, &bad).is_err(), "{bad}");
        }
    }

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
