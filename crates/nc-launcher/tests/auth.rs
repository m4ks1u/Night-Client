//! Authentication chain and loopback listener error handling (mock services).

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use nc_core::Secret;
use nc_launcher::{
    CancellationToken,
    auth::{AuthConfig, AuthError, Client, Loopback, MsTokens},
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header, method, path},
};

const MS_TOKEN: &str = "ms-access-SECRET-1";
const MC_TOKEN: &str = "mc-access-SECRET-2";

async fn server_with_happy_path() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/xbl"))
        .and(body_string_contains(format!("d={MS_TOKEN}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"Token":"xbl-tok","DisplayClaims":{"xui":[{"uhs":"UHS1"}]}})),
        )
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/xsts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"Token":"xsts-tok","DisplayClaims":{"xui":[{"uhs":"UHS1","xid":"2535"}]}}),
        ))
        .with_priority(5)
        .mount(&s)
        .await;
    Mock::given(method("POST")).and(path("/mc/login")).and(body_string_contains("XBL3.0 x=UHS1;xsts-tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username":"u","access_token":MC_TOKEN,"expires_in":86400,"token_type":"Bearer"})))
        .with_priority(5).mount(&s).await;
    Mock::given(method("GET"))
        .and(path("/mc/entitlements"))
        .and(header(
            "authorization",
            format!("Bearer {MC_TOKEN}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"items":[{"name":"product_minecraft"},{"name":"game_minecraft"}]}),
        ))
        .with_priority(5)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/mc/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"0123456789abcdef0123456789abcdef","name":"Steve","skins":[]}),
        ))
        .with_priority(5)
        .mount(&s)
        .await;
    s
}

fn client(s: &MockServer) -> Client {
    Client::new(
        AuthConfig::with_base("test-client", &s.uri()),
        "night-client-tests",
    )
    .unwrap()
}

fn ms() -> MsTokens {
    MsTokens {
        access_token: Secret::new(MS_TOKEN),
        refresh_token: Some(Secret::new("refresh-SECRET-3")),
    }
}

#[tokio::test]
async fn happy_path_yields_profile_and_redacted_session() {
    let s = server_with_happy_path().await;
    let session = client(&s).minecraft_session(ms()).await.unwrap();
    assert_eq!(session.profile.name, "Steve");
    assert_eq!(session.access_token.expose(), MC_TOKEN);
    assert_eq!(session.xuid.as_deref(), Some("2535"));
    let dbg = format!("{session:?}");
    for secret in [MC_TOKEN, "refresh-SECRET-3"] {
        assert!(!dbg.contains(secret), "Debug output leaked a token");
    }
}

async fn xsts_error(code: u64) -> AuthError {
    let s = server_with_happy_path().await;
    Mock::given(path("/xsts")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"Identity":"0","XErr":code,"Message":"","Redirect":"https://start.ui.xboxlive.com/CreateAccount"})))
        .with_priority(1).mount(&s).await;
    client(&s).minecraft_session(ms()).await.unwrap_err()
}

#[tokio::test]
async fn xsts_errors_map_to_actionable_messages() {
    assert!(matches!(
        xsts_error(2148916233).await,
        AuthError::NoXboxAccount
    ));
    assert!(matches!(
        xsts_error(2148916235).await,
        AuthError::XboxRegionUnavailable
    ));
    assert!(matches!(
        xsts_error(2148916236).await,
        AuthError::AdultVerificationRequired
    ));
    assert!(matches!(
        xsts_error(2148916238).await,
        AuthError::ChildAccount
    ));
    assert!(matches!(
        xsts_error(1).await,
        AuthError::Http {
            stage: "xsts",
            status: 401
        }
    ));
}

#[tokio::test]
async fn unapproved_app_id_is_detected() {
    let s = server_with_happy_path().await;
    Mock::given(path("/mc/login"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"error":"Invalid app registration"})),
        )
        .with_priority(1)
        .mount(&s)
        .await;
    let e = client(&s).minecraft_session(ms()).await.unwrap_err();
    assert!(matches!(e, AuthError::AppNotApproved));
    assert_eq!(e.code(), "auth_app_not_approved");
}

#[tokio::test]
async fn account_without_game_is_rejected() {
    let s = server_with_happy_path().await;
    Mock::given(path("/mc/entitlements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[]})))
        .with_priority(1)
        .mount(&s)
        .await;
    assert!(matches!(
        client(&s).minecraft_session(ms()).await.unwrap_err(),
        AuthError::NotOwned
    ));
}

#[tokio::test]
async fn owner_without_profile_is_reported() {
    let s = server_with_happy_path().await;
    Mock::given(path("/mc/profile"))
        .respond_with(ResponseTemplate::new(404))
        .with_priority(1)
        .mount(&s)
        .await;
    assert!(matches!(
        client(&s).minecraft_session(ms()).await.unwrap_err(),
        AuthError::NoProfile
    ));
}

#[tokio::test]
async fn expired_refresh_token_requires_reauth() {
    let s = MockServer::start().await;
    Mock::given(path("/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":"invalid_grant","error_description":"AADSTS70000"})),
        )
        .mount(&s)
        .await;
    let e = client(&s).refresh(&Secret::new("old")).await.err().unwrap();
    assert!(matches!(e, AuthError::ReauthRequired));
}

#[tokio::test]
async fn refresh_success_returns_rotated_tokens() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("client_id=test-client"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"new-a","refresh_token":"new-r","expires_in":3600,"token_type":"Bearer"})))
        .mount(&s).await;
    let t = client(&s).refresh(&Secret::new("old")).await.ok().unwrap();
    assert_eq!(t.access_token.expose(), "new-a");
    assert_eq!(t.refresh_token.unwrap().expose(), "new-r");
}

#[tokio::test]
async fn token_rate_limit_is_reported() {
    let s = MockServer::start().await;
    Mock::given(path("/token"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&s)
        .await;
    assert!(matches!(
        client(&s).refresh(&Secret::new("x")).await.err().unwrap(),
        AuthError::RateLimited
    ));
}

async fn browser_hit(redirect: &str, query: &str) -> String {
    let url = format!("{}/?{query}", redirect.replace("localhost", "127.0.0.1"));
    reqwest::get(url).await.unwrap().text().await.unwrap()
}

#[tokio::test]
async fn loopback_returns_code_and_ignores_favicon() {
    let lb = Loopback::bind().await.unwrap();
    let redirect = lb.redirect_uri.clone();
    assert!(redirect.starts_with("http://localhost:"));
    let h = tokio::spawn(async move {
        lb.wait_for_code("st8", &CancellationToken::new(), Duration::from_secs(5))
            .await
    });
    let _ = reqwest::get(format!(
        "{}/favicon.ico",
        redirect.replace("localhost", "127.0.0.1")
    ))
    .await;
    let page = browser_hit(&redirect, "code=abc&state=st8").await;
    assert!(page.contains("Signed in"));
    assert_eq!(h.await.unwrap().unwrap().expose(), "abc");
}

#[tokio::test]
async fn loopback_rejects_state_mismatch() {
    let lb = Loopback::bind().await.unwrap();
    let redirect = lb.redirect_uri.clone();
    let h = tokio::spawn(async move {
        lb.wait_for_code(
            "expected",
            &CancellationToken::new(),
            Duration::from_secs(5),
        )
        .await
    });
    browser_hit(&redirect, "code=abc&state=forged").await;
    assert!(matches!(h.await.unwrap(), Err(AuthError::StateMismatch)));
}

#[tokio::test]
async fn loopback_reports_user_denial() {
    let lb = Loopback::bind().await.unwrap();
    let redirect = lb.redirect_uri.clone();
    let h = tokio::spawn(async move {
        lb.wait_for_code("s", &CancellationToken::new(), Duration::from_secs(5))
            .await
    });
    browser_hit(&redirect, "error=access_denied&state=s").await;
    assert!(matches!(h.await.unwrap(), Err(AuthError::Denied(e)) if e == "access_denied"));
}

#[tokio::test]
async fn loopback_times_out_and_cancels() {
    let lb = Loopback::bind().await.unwrap();
    assert!(matches!(
        lb.wait_for_code("s", &CancellationToken::new(), Duration::from_millis(100))
            .await,
        Err(AuthError::Timeout)
    ));
    let lb = Loopback::bind().await.unwrap();
    let c = CancellationToken::new();
    c.cancel();
    assert!(matches!(
        lb.wait_for_code("s", &c, Duration::from_secs(5)).await,
        Err(AuthError::Cancelled)
    ));
}
