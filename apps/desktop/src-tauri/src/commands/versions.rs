use nc_core::manifest::VersionKind;
use nc_launcher::CancellationToken;
use serde::Serialize;
use tauri::State;

use super::Result;
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionItem {
    id: String,
    kind: VersionKind,
    release_time: String,
    installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionList {
    latest_release: String,
    latest_snapshot: String,
    versions: Vec<VersionItem>,
    from_cache: bool,
}

#[tauri::command]
pub async fn list_versions(
    state: State<'_, AppState>,
    include_snapshots: bool,
) -> Result<VersionList> {
    let (m, from_cache) = state
        .meta()
        .version_manifest(&CancellationToken::new())
        .await?;
    let layout = state.layout.clone();
    let versions = m
        .versions
        .into_iter()
        .filter(|v| {
            v.kind == VersionKind::Release || (include_snapshots && v.kind == VersionKind::Snapshot)
        })
        .map(|v| VersionItem {
            installed: layout.installed_marker(&v.id).is_ok_and(|p| p.is_file()),
            id: v.id,
            kind: v.kind,
            release_time: v.release_time,
        })
        .collect();
    Ok(VersionList {
        latest_release: m.latest.release,
        latest_snapshot: m.latest.snapshot,
        versions,
        from_cache,
    })
}
