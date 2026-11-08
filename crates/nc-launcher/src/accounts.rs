//! Persistent Microsoft accounts: refresh-token storage and session renewal.
//!
//! Only the Microsoft refresh token is persisted (DPAPI vault, keyed by the
//! Minecraft profile UUID). Minecraft access tokens live in memory and are
//! re-derived through the full Xbox chain when they expire, so entitlement
//! and profile are re-checked on every renewal.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    LauncherError,
    auth::{AuthError, Client, MinecraftSession},
    vault::TokenVault,
};

/// Renew Minecraft tokens this long before they expire.
pub const REFRESH_MARGIN_SECS: u64 = 300;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn needs_refresh(session: &MinecraftSession) -> bool {
    session.expires_at_unix <= now() + REFRESH_MARGIN_SECS
}

fn vault_err(e: std::io::Error) -> LauncherError {
    LauncherError::Vault(e.to_string())
}

/// Store a freshly signed-in session's refresh token.
pub fn persist(vault: &dyn TokenVault, session: &MinecraftSession) -> Result<(), LauncherError> {
    let rt = session
        .ms_refresh_token
        .as_ref()
        .ok_or(AuthError::Protocol {
            stage: "microsoft token",
            message: "no refresh token was issued (offline_access scope missing?)".into(),
        })?;
    vault.put(&session.profile.id, rt).map_err(vault_err)
}

/// Remove an account's stored credentials.
pub fn forget(vault: &dyn TokenVault, uuid: &str) -> Result<(), LauncherError> {
    vault.delete(uuid).map_err(vault_err)
}

/// Get a valid Minecraft session for `uuid` from its stored refresh token.
///
/// * The rotated refresh token Microsoft returns is saved immediately.
/// * `invalid_grant` (revoked or expired) deletes the stored token and
///   returns `ReauthRequired`.
/// * The renewed session must belong to the same profile.
pub async fn refresh(
    client: &Client,
    vault: &dyn TokenVault,
    uuid: &str,
) -> Result<MinecraftSession, LauncherError> {
    let rt = vault
        .get(uuid)
        .map_err(vault_err)?
        .ok_or(AuthError::ReauthRequired)?;
    let ms = match client.refresh(&rt).await {
        Ok(ms) => ms,
        Err(AuthError::ReauthRequired) => {
            let _ = vault.delete(uuid);
            return Err(AuthError::ReauthRequired.into());
        }
        Err(e) => return Err(e.into()),
    };
    // Save the rotated token before the rest of the chain: the old one may
    // already be invalid, and a later failure must not lose the new one.
    if let Some(new_rt) = &ms.refresh_token {
        vault.put(uuid, new_rt).map_err(vault_err)?;
    }
    let session = client.minecraft_session(ms).await?;
    if !session.profile.id.eq_ignore_ascii_case(uuid) {
        return Err(AuthError::AccountMismatch.into());
    }
    tracing::info!(player = %session.profile.name, "session renewed");
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth::Profile, vault::MemoryVault};
    use nc_core::Secret;

    #[test]
    fn refresh_margin() {
        let mut s = MinecraftSession {
            profile: Profile {
                id: "0".repeat(32),
                name: "x".into(),
            },
            access_token: Secret::new("t"),
            expires_at_unix: now() + 3600,
            xuid: None,
            ms_refresh_token: None,
        };
        assert!(!needs_refresh(&s));
        s.expires_at_unix = now() + 60;
        assert!(needs_refresh(&s));
        assert!(
            persist(&MemoryVault::default(), &s).is_err(),
            "no refresh token to persist"
        );
    }
}
