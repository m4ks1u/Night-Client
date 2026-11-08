//! Microsoft accounts: sign-in, persistence (DPAPI), switching, removal and
//! session renewal. No token ever crosses IPC.

use std::time::Duration;

use nc_launcher::{CancellationToken, LauncherError, accounts, auth::MinecraftSession};
use nc_store::AccountRecord;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::{CmdError, Result, with_db};
use crate::state::{AppState, lock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    /// A live session is in memory.
    Ready,
    /// A stored sign-in exists; it is renewed on first use.
    SignedIn,
    /// No usable stored sign-in: the user must sign in again.
    Expired,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    uuid: String,
    name: String,
    active: bool,
    last_used_at: Option<i64>,
    status: AccountStatus,
}

fn status(state: &AppState, uuid: &str) -> AccountStatus {
    if lock(&state.sessions)
        .get(uuid)
        .is_some_and(|s| !accounts::needs_refresh(s))
    {
        AccountStatus::Ready
    } else if state.vault.get(uuid).ok().flatten().is_some() {
        AccountStatus::SignedIn
    } else {
        AccountStatus::Expired
    }
}

fn view(state: &AppState, r: AccountRecord) -> AccountView {
    AccountView {
        status: status(state, &r.uuid),
        uuid: r.uuid,
        name: r.name,
        active: r.active,
        last_used_at: r.last_used_at,
    }
}

fn validate_uuid(uuid: &str) -> Result<()> {
    if uuid.len() == 32 && uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(super::invalid("invalid account id"))
    }
}

/// A valid session for the active account, renewing it from the vault when
/// needed. Used by launch and by the UI's "check sign-in" call.
pub async fn active_session(state: &AppState) -> Result<MinecraftSession> {
    let active = with_db(state, |db| db.active_account())
        .await?
        .ok_or(CmdError::new(
            "auth_required",
            "Sign in with Microsoft to play.",
        ))?;
    if let Some(s) = lock(&state.sessions).get(&active.uuid)
        && !accounts::needs_refresh(s)
    {
        return Ok(s.clone());
    }
    let reauth = || {
        CmdError::new(
            "auth_reauth_required",
            format!(
                "The saved sign-in for {} has expired. Sign in again.",
                active.name
            ),
        )
    };
    if state.vault.get(&active.uuid).ok().flatten().is_none() {
        return Err(reauth());
    }
    let client = state.auth.as_ref().ok_or(CmdError::new(
        "auth_not_configured",
        "Microsoft sign-in is not configured in this build (NIGHT_CLIENT_MSA_CLIENT_ID).",
    ))?;
    let _guard = state.refresh_lock.lock().await;
    // Another caller may have renewed it while we waited.
    if let Some(s) = lock(&state.sessions).get(&active.uuid)
        && !accounts::needs_refresh(s)
    {
        return Ok(s.clone());
    }
    match accounts::refresh(client, state.vault.as_ref(), &active.uuid).await {
        Ok(session) => {
            if session.profile.name != active.name {
                let (u, n) = (active.uuid.clone(), session.profile.name.clone());
                let _ = with_db(state, move |db| db.upsert_active_account(&u, &n)).await;
            }
            lock(&state.sessions).insert(active.uuid.clone(), session.clone());
            Ok(session)
        }
        Err(LauncherError::Auth(nc_launcher::auth::AuthError::ReauthRequired)) => {
            lock(&state.sessions).remove(&active.uuid);
            Err(reauth())
        }
        Err(e) => Err(e.into()),
    }
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>> {
    let list = with_db(&state, |db| db.list_accounts()).await?;
    Ok(list.into_iter().map(|r| view(&state, r)).collect())
}

/// The active account, if any.
#[tauri::command]
pub async fn account(state: State<'_, AppState>) -> Result<Option<AccountView>> {
    let a = with_db(&state, |db| db.active_account()).await?;
    Ok(a.map(|r| view(&state, r)))
}

/// Renew the active account's session now (used at startup and after
/// switching) so problems surface before the user presses Play.
#[tauri::command]
pub async fn check_active_account(state: State<'_, AppState>) -> Result<Option<AccountView>> {
    match active_session(&state).await {
        Ok(_)
        | Err(CmdError {
            code: "auth_required",
            ..
        }) => {}
        Err(e) => tracing::warn!(code = e.code, "active account check failed"),
    }
    account(state).await
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

    accounts::persist(state.vault.as_ref(), &session)?;
    let (uuid, name) = (
        session.profile.id.to_ascii_lowercase(),
        session.profile.name.clone(),
    );
    let record = {
        let (u, n) = (uuid.clone(), name.clone());
        with_db(&state, move |db| db.upsert_active_account(&u, &n)).await?
    };
    lock(&state.sessions).insert(uuid, session);
    tracing::info!(player = %name, "signed in and saved");
    Ok(view(&state, record))
}

#[tauri::command]
pub fn cancel_sign_in(state: State<'_, AppState>) {
    if let Some(t) = lock(&state.sign_in).as_ref() {
        t.cancel();
    }
}

#[tauri::command]
pub async fn switch_account(state: State<'_, AppState>, uuid: String) -> Result<AccountView> {
    validate_uuid(&uuid)?;
    if !lock(&state.games).is_empty() {
        return Err(CmdError::new(
            "busy",
            "close running games before switching accounts",
        ));
    }
    let r = with_db(&state, move |db| db.set_active_account(&uuid)).await?;
    tracing::info!(player = %r.name, "switched account");
    Ok(view(&state, r))
}

/// Forget an account: delete its stored token and metadata. Returns the new
/// active account, if any.
#[tauri::command]
pub async fn remove_account(
    state: State<'_, AppState>,
    uuid: String,
) -> Result<Option<AccountView>> {
    validate_uuid(&uuid)?;
    let key = uuid.to_ascii_lowercase();
    accounts::forget(state.vault.as_ref(), &key)?;
    lock(&state.sessions).remove(&key);
    let next = with_db(&state, move |db| db.remove_account(&key)).await?;
    tracing::info!("account removed");
    Ok(next.map(|r| view(&state, r)))
}
