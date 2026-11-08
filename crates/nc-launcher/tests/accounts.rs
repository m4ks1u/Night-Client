//! Session renewal from the vault against mock Microsoft/Xbox/Minecraft services.

#![allow(clippy::unwrap_used)]

use nc_core::Secret;
use nc_launcher::{
    LauncherError, accounts,
    auth::{AuthConfig, AuthError, Client},
    vault::{MemoryVault, TokenVault},
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

const UUID: &str = "0123456789abcdef0123456789abcdef";

async fn services(profile_id: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=old-rt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"ms-a","refresh_token":"rotated-rt","expires_in":3600}),
        ))
        .mount(&s)
        .await;
    Mock::given(path("/xbl"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"Token":"x","DisplayClaims":{"xui":[{"uhs":"U"}]}})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/xsts"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"Token":"y","DisplayClaims":{"xui":[{"uhs":"U"}]}})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/mc/login"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"mc-a","expires_in":86400})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/mc/entitlements"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"items":[{"name":"game_minecraft"}]})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/mc/profile"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":profile_id,"name":"Steve"})),
        )
        .mount(&s)
        .await;
    s
}

fn client(s: &MockServer) -> Client {
    Client::new(AuthConfig::with_base("cid", &s.uri()), "test").unwrap()
}

#[tokio::test]
async fn refresh_renews_session_and_stores_rotated_token() {
    let s = services(UUID).await;
    let vault = MemoryVault::default();
    vault.put(UUID, &Secret::new("old-rt")).unwrap();
    let session = accounts::refresh(&client(&s), &vault, UUID).await.unwrap();
    assert_eq!(session.profile.name, "Steve");
    assert_eq!(session.access_token.expose(), "mc-a");
    assert!(!accounts::needs_refresh(&session));
    assert_eq!(vault.get(UUID).unwrap().unwrap().expose(), "rotated-rt");
}

#[tokio::test]
async fn revoked_refresh_token_is_deleted_and_requires_sign_in() {
    let s = MockServer::start().await;
    Mock::given(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .mount(&s)
        .await;
    let vault = MemoryVault::default();
    vault.put(UUID, &Secret::new("revoked")).unwrap();
    let err = accounts::refresh(&client(&s), &vault, UUID)
        .await
        .unwrap_err();
    assert!(
        matches!(err, LauncherError::Auth(AuthError::ReauthRequired)),
        "{err}"
    );
    assert_eq!(err.code(), "auth_reauth_required");
    assert!(vault.get(UUID).unwrap().is_none(), "dead token removed");
}

#[tokio::test]
async fn missing_token_requires_sign_in_without_network() {
    let s = MockServer::start().await; // no mocks: any request would 404
    let err = accounts::refresh(&client(&s), &MemoryVault::default(), UUID)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LauncherError::Auth(AuthError::ReauthRequired)
    ));
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn token_for_another_profile_is_rejected() {
    let s = services("ffffffffffffffffffffffffffffffff").await;
    let vault = MemoryVault::default();
    vault.put(UUID, &Secret::new("old-rt")).unwrap();
    let err = accounts::refresh(&client(&s), &vault, UUID)
        .await
        .unwrap_err();
    assert!(
        matches!(err, LauncherError::Auth(AuthError::AccountMismatch)),
        "{err}"
    );
}

#[tokio::test]
async fn rotated_token_survives_a_later_chain_failure() {
    let s = MockServer::start().await;
    Mock::given(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"ms-a","refresh_token":"rotated-rt"})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/xbl"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&s)
        .await;
    let vault = MemoryVault::default();
    vault.put(UUID, &Secret::new("old-rt")).unwrap();
    assert!(accounts::refresh(&client(&s), &vault, UUID).await.is_err());
    assert_eq!(vault.get(UUID).unwrap().unwrap().expose(), "rotated-rt");
}

#[tokio::test]
async fn persist_and_forget() {
    let s = services(UUID).await;
    let vault = MemoryVault::default();
    vault.put(UUID, &Secret::new("old-rt")).unwrap();
    let session = accounts::refresh(&client(&s), &vault, UUID).await.unwrap();
    vault.delete(UUID).unwrap();
    // A fresh interactive sign-in would call persist with a session that has a refresh token.
    let mut with_rt = session.clone();
    with_rt.ms_refresh_token = Some(Secret::new("new-login-rt"));
    accounts::persist(&vault, &with_rt).unwrap();
    assert_eq!(vault.get(UUID).unwrap().unwrap().expose(), "new-login-rt");
    accounts::forget(&vault, UUID).unwrap();
    assert!(vault.get(UUID).unwrap().is_none());
}
