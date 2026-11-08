use std::path::PathBuf;

use nc_core::java::{Compatibility, PeArch, compatibility};
use nc_launcher::javadetect::{self, JavaRuntime, JavaSource};
use nc_store::InstancePatch;
use serde::Serialize;
use tauri::State;

use super::{CmdError, Result, validate_instance_id, with_db};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaView {
    #[serde(flatten)]
    runtime: JavaRuntime,
    /// Against the requested version, when one was given.
    compatibility: Option<Compatibility>,
}

/// Required Java major for an installed (or at least downloaded) version.
async fn required_major(state: &AppState, version: &str) -> Option<u32> {
    let path = state.layout.version_json(version).ok()?;
    let v: nc_core::manifest::VersionJson =
        nc_launcher::meta::read_json(&path, "version").await.ok()?;
    Some(nc_core::java::requirement(&v).major_version)
}

fn view(rt: JavaRuntime, required: Option<u32>) -> JavaView {
    let compatibility = required.map(|r| compatibility(r, rt.major, rt.arch == PeArch::X64));
    JavaView {
        runtime: rt,
        compatibility,
    }
}

/// Scan the machine for Java runtimes; optionally rate them for a version.
#[tauri::command]
pub async fn detect_java(
    state: State<'_, AppState>,
    for_version: Option<String>,
) -> Result<Vec<JavaView>> {
    if let Some(v) = &for_version {
        super::validate_version_id(v)?;
    }
    let required = match &for_version {
        Some(v) => required_major(&state, v).await,
        None => None,
    };
    let layout = state.layout.clone();
    let found = tauri::async_runtime::spawn_blocking(move || javadetect::detect(&layout))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?;
    tracing::info!(count = found.len(), "Java scan finished");
    Ok(found.into_iter().map(|r| view(r, required)).collect())
}

/// Set (or clear with `null`) an instance's Java. The runtime is inspected
/// and must be 64-bit and new enough for the instance's version.
#[tauri::command]
pub async fn set_instance_java(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
) -> Result<Option<JavaView>> {
    validate_instance_id(&id)?;
    let lookup = id.clone();
    let instance = with_db(&state, move |db| db.get_instance(&lookup)).await?;

    let (stored, view_out) = match path {
        None => (None, None),
        Some(p) => {
            if p.len() > 1024 {
                return Err(super::invalid("path is too long"));
            }
            // Inspecting a runtime executes `java -version`, so never run a
            // binary from a network share or a raw device path.
            if p.starts_with(r"\\") || p.starts_with("//") {
                return Err(super::invalid(
                    "Java must be on a local drive, not a network or device path",
                ));
            }
            let input = PathBuf::from(&p);
            if !input.is_absolute() {
                return Err(super::invalid(
                    "choose an absolute path to java.exe or javaw.exe",
                ));
            }
            let rt = tauri::async_runtime::spawn_blocking(move || {
                javadetect::inspect(&input, JavaSource::Custom)
            })
            .await
            .map_err(|e| CmdError::new("internal", e.to_string()))?
            .map_err(|e| CmdError::new("java_invalid", e.to_string()))?;
            let required = required_major(&state, &instance.game_version).await;
            let v = view(rt, required);
            if v.runtime.arch != PeArch::X64 {
                return Err(CmdError::new(
                    "java_incompatible",
                    "Night Client needs a 64-bit (x64) Java runtime.",
                ));
            }
            if v.compatibility == Some(Compatibility::Incompatible) {
                return Err(CmdError::new(
                    "java_incompatible",
                    format!(
                        "Minecraft {} needs Java {} or newer; this runtime is Java {}.",
                        instance.game_version,
                        required.unwrap_or(0),
                        v.runtime.major
                    ),
                ));
            }
            (Some(v.runtime.path.display().to_string()), Some(v))
        }
    };
    let patch = InstancePatch {
        java_path: Some(stored),
        ..Default::default()
    };
    with_db(&state, move |db| db.update_instance(&id, patch)).await?;
    Ok(view_out)
}
