use std::time::Duration;

use nc_launcher::CancellationToken;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::{CmdError, Result};
use crate::state::{AppState, lock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    name: String,
    uuid: String,
    expires_at: u64,
}

#[tauri::command]
pub fn account(state: State<'_, AppState>) -> Option<AccountView> {
    lock(&state.session).as_ref().map(|s| AccountView {
        name: s.profile.name.clone(),
        uuid: s.profile.id.clone(),
        expires_at: s.expires_at_unix,
    })
}

#[tauri::command]
pub async fn sign_in(app: AppHandle, state: State<'_, AppState>) -> Result<AccountView> {
    let Some(client) = state.auth.as_ref() else {
        return Err(CmdError::new(
            "auth_not_configured",
            "Microsoft sign-in is not configured in this build (NIGHT_CLIENT_MSA_CLIENT_ID).",
        ));
    };
    let token = {
        let mut slot = lock(&state.sign_in);
        if slot.is_some() {
            return Err(CmdError::new("busy", "a sign-in is already in progress"));
        }
        let t = CancellationToken::new();
        *slot = Some(t.clone());
        t
    };
    let opener = app.clone();
    let r = client
        .login_interactive(
            move |url| {
                opener
                    .opener()
                    .open_url(url, None::<&str>)
                    .map_err(|e| e.to_string())
            },
            &token,
            Duration::from_secs(300),
        )
        .await;
    *lock(&state.sign_in) = None;
    let session = r.inspect_err(|e| tracing::warn!(code = e.code(), "sign-in failed"))?;
    let view = AccountView {
        name: session.profile.name.clone(),
        uuid: session.profile.id.clone(),
        expires_at: session.expires_at_unix,
    };
    tracing::info!(player = %view.name, "signed in");
    *lock(&state.session) = Some(session);
    Ok(view)
}

#[tauri::command]
pub fn cancel_sign_in(state: State<'_, AppState>) {
    if let Some(t) = lock(&state.sign_in).as_ref() {
        t.cancel();
    }
}

#[tauri::command]
pub fn sign_out(state: State<'_, AppState>) {
    if lock(&state.session).take().is_some() {
        tracing::info!("signed out");
    }
}
