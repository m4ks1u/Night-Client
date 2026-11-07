//! Microsoft → Xbox Live → XSTS → Minecraft authentication.
//!
//! The Microsoft step uses the OAuth 2.0 authorization-code flow with PKCE
//! (S256) in the user's system browser, returning to a one-shot loopback
//! listener (RFC 8252 §7.3). No client secret exists or is needed.
//!
//! Tokens are wrapped in [`Secret`] so they cannot reach logs through
//! `Debug`/`Display`. Persisting the refresh token is the caller's job and
//! must use OS-protected storage (DPAPI), never SQLite or plain files.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nc_core::Secret;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

pub const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub xbl_url: String,
    pub xsts_url: String,
    pub mc_login_url: String,
    pub mc_entitlements_url: String,
    pub mc_profile_url: String,
}

impl AuthConfig {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            authorize_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize"
                .into(),
            token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".into(),
            xbl_url: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts_url: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            mc_login_url: "https://api.minecraftservices.com/authentication/login_with_xbox".into(),
            mc_entitlements_url: "https://api.minecraftservices.com/entitlements/mcstore".into(),
            mc_profile_url: "https://api.minecraftservices.com/minecraft/profile".into(),
        }
    }

    /// Point every endpoint at one base URL (tests with a mock server).
    pub fn with_base(client_id: &str, base: &str) -> Self {
        Self {
            client_id: client_id.into(),
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            xbl_url: format!("{base}/xbl"),
            xsts_url: format!("{base}/xsts"),
            mc_login_url: format!("{base}/mc/login"),
            mc_entitlements_url: format!("{base}/mc/entitlements"),
            mc_profile_url: format!("{base}/mc/profile"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("sign-in was cancelled")]
    Cancelled,
    #[error("sign-in timed out")]
    Timeout,
    #[error("Microsoft sign-in was declined: {0}")]
    Denied(String),
    #[error("sign-in response did not match this request (state mismatch)")]
    StateMismatch,
    #[error("this Microsoft account has no Xbox profile yet; create one at xbox.com and try again")]
    NoXboxAccount,
    #[error("Xbox Live is not available in this account's country or region")]
    XboxRegionUnavailable,
    #[error("this account needs age verification on xbox.com before it can play")]
    AdultVerificationRequired,
    #[error("this is a child account; an adult must add it to a Microsoft family group")]
    ChildAccount,
    #[error("Night Client's application ID has not been approved for Minecraft sign-in yet")]
    AppNotApproved,
    #[error("this account does not own Minecraft: Java Edition")]
    NotOwned,
    #[error("this account owns Minecraft but has not created a profile (player name) yet")]
    NoProfile,
    #[error("the saved sign-in expired; please sign in again")]
    ReauthRequired,
    #[error("too many sign-in attempts; wait a minute and try again")]
    RateLimited,
    #[error("{stage}: unexpected HTTP {status}")]
    Http { stage: &'static str, status: u16 },
    #[error("{stage}: network error: {message}")]
    Network {
        stage: &'static str,
        message: String,
    },
    #[error("{stage}: unexpected response: {message}")]
    Protocol {
        stage: &'static str,
        message: String,
    },
    #[error("loopback listener: {0}")]
    Listener(String),
}

impl AuthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout => "auth_timeout",
            Self::Denied(_) => "auth_denied",
            Self::StateMismatch => "auth_state_mismatch",
            Self::NoXboxAccount => "auth_no_xbox_account",
            Self::XboxRegionUnavailable => "auth_region",
            Self::AdultVerificationRequired => "auth_age_verification",
            Self::ChildAccount => "auth_child_account",
            Self::AppNotApproved => "auth_app_not_approved",
            Self::NotOwned => "auth_not_owned",
            Self::NoProfile => "auth_no_profile",
            Self::ReauthRequired => "auth_reauth_required",
            Self::RateLimited => "auth_rate_limited",
            Self::Http { .. } | Self::Network { .. } | Self::Protocol { .. } => "auth_service",
            Self::Listener(_) => "auth_listener",
        }
    }
}

fn net(stage: &'static str) -> impl FnOnce(reqwest::Error) -> AuthError {
    // Strip URLs: some endpoints echo query parameters.
    move |e| AuthError::Network {
        stage,
        message: e.without_url().to_string(),
    }
}

fn proto(stage: &'static str) -> impl FnOnce(reqwest::Error) -> AuthError {
    move |e| AuthError::Protocol {
        stage,
        message: e.without_url().to_string(),
    }
}

// ---------------------------------------------------------------- PKCE ----

pub struct Pkce {
    pub verifier: Secret,
    pub challenge: String,
}

fn random_b64(len: usize) -> Result<String, AuthError> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf)
        .map_err(|e| AuthError::Listener(format!("OS RNG unavailable: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

impl Pkce {
    pub fn generate() -> Result<Self, AuthError> {
        let verifier = random_b64(32)?; // 43 chars, within RFC 7636's 43..=128
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier: Secret::new(verifier),
            challenge,
        })
    }
}

pub fn authorize_url(cfg: &AuthConfig, redirect_uri: &str, pkce: &Pkce, state: &str) -> String {
    let mut u = url::Url::parse(&cfg.authorize_url).expect("static authorize URL is valid");
    u.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    u.into()
}

// ------------------------------------------------------------ loopback ----

/// One-shot HTTP listener on 127.0.0.1 for the OAuth redirect.
pub struct Loopback {
    listener: TcpListener,
    pub redirect_uri: String,
}

impl Loopback {
    pub async fn bind() -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| AuthError::Listener(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| AuthError::Listener(e.to_string()))?
            .port();
        // Entra ID matches `http://localhost` registrations for any port.
        Ok(Self {
            listener,
            redirect_uri: format!("http://localhost:{port}"),
        })
    }

    /// Wait for the browser redirect and return the authorization code.
    pub async fn wait_for_code(
        self,
        expected_state: &str,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<Secret, AuthError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let (mut sock, _) = tokio::select! {
                r = self.listener.accept() => r.map_err(|e| AuthError::Listener(e.to_string()))?,
                _ = cancel.cancelled() => return Err(AuthError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => return Err(AuthError::Timeout),
            };
            let request =
                match tokio::time::timeout(Duration::from_secs(5), read_head(&mut sock)).await {
                    Ok(Ok(r)) => r,
                    _ => continue, // slow or broken client; keep listening
                };
            let Some(query) = parse_request_query(&request) else {
                let _ = respond(&mut sock, 404, "Not found").await;
                continue;
            };
            let params: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(query.as_bytes())
                    .into_owned()
                    .collect();

            if params.get("state").map(String::as_str) != Some(expected_state) {
                let _ = respond(
                    &mut sock,
                    400,
                    "This sign-in link is not valid. Return to Night Client and try again.",
                )
                .await;
                return Err(AuthError::StateMismatch);
            }
            if let Some(err) = params.get("error") {
                let _ = respond(
                    &mut sock,
                    200,
                    "Sign-in was cancelled. You can close this tab.",
                )
                .await;
                return Err(AuthError::Denied(err.clone()));
            }
            if let Some(code) = params.get("code") {
                let _ = respond(
                    &mut sock,
                    200,
                    "Signed in. You can close this tab and return to Night Client.",
                )
                .await;
                return Ok(Secret::new(code.clone()));
            }
            let _ = respond(&mut sock, 400, "Missing authorization code.").await;
        }
    }
}

async fn read_head(sock: &mut tokio::net::TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Returns the query string of `GET /?...`; other paths (e.g. favicon) yield `None`.
fn parse_request_query(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    let (path, query) = target.split_once('?')?;
    (path == "/").then(|| query.to_owned())
}

async fn respond(
    sock: &mut tokio::net::TcpStream,
    status: u16,
    message: &str,
) -> std::io::Result<()> {
    let reason = if status == 200 {
        "OK"
    } else if status == 404 {
        "Not Found"
    } else {
        "Bad Request"
    };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Night Client</title>\
         <body style=\"background:#0b0b10;color:#e7e7ee;font:16px system-ui;display:grid;place-items:center;height:100vh;margin:0\">\
         <p>{}</p>",
        html_escape(message)
    );
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    sock.write_all(resp.as_bytes()).await?;
    sock.shutdown().await
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// -------------------------------------------------------- token chain ----

pub struct MsTokens {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
}

pub struct Client {
    http: reqwest::Client,
    cfg: AuthConfig,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Profile {
    /// UUID without dashes.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MinecraftSession {
    pub profile: Profile,
    pub access_token: Secret,
    pub expires_at_unix: u64,
    pub xuid: Option<String>,
    /// Long-lived Microsoft refresh token: persist with DPAPI only.
    pub ms_refresh_token: Option<Secret>,
}

impl Client {
    pub fn new(cfg: AuthConfig, user_agent: &str) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AuthError::Network {
                stage: "client",
                message: e.to_string(),
            })?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &AuthConfig {
        &self.cfg
    }

    /// Full interactive sign-in. `open_browser` receives the authorization URL.
    pub async fn login_interactive(
        &self,
        open_browser: impl FnOnce(&str) -> Result<(), String>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<MinecraftSession, AuthError> {
        let loopback = Loopback::bind().await?;
        let pkce = Pkce::generate()?;
        let state = random_b64(16)?;
        let redirect = loopback.redirect_uri.clone();
        open_browser(&authorize_url(&self.cfg, &redirect, &pkce, &state))
            .map_err(AuthError::Listener)?;
        let code = loopback.wait_for_code(&state, cancel, timeout).await?;
        let ms = self.exchange_code(&code, &redirect, &pkce).await?;
        self.minecraft_session(ms).await
    }

    pub async fn exchange_code(
        &self,
        code: &Secret,
        redirect_uri: &str,
        pkce: &Pkce,
    ) -> Result<MsTokens, AuthError> {
        self.token_request(&[
            ("client_id", self.cfg.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code.expose()),
            ("redirect_uri", redirect_uri),
            ("code_verifier", pkce.verifier.expose()),
            ("scope", SCOPE),
        ])
        .await
    }

    pub async fn refresh(&self, refresh_token: &Secret) -> Result<MsTokens, AuthError> {
        self.token_request(&[
            ("client_id", self.cfg.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("scope", SCOPE),
        ])
        .await
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<MsTokens, AuthError> {
        const STAGE: &str = "microsoft token";
        let resp = self
            .http
            .post(&self.cfg.token_url)
            .form(form)
            .send()
            .await
            .map_err(net(STAGE))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(AuthError::RateLimited);
        }
        if !status.is_success() {
            let err: Option<OAuthError> = resp.json().await.ok();
            return Err(match err.map(|e| e.error).as_deref() {
                Some("invalid_grant") => AuthError::ReauthRequired,
                Some(e @ ("access_denied" | "consent_required" | "interaction_required")) => {
                    AuthError::Denied(e.into())
                }
                _ => AuthError::Http {
                    stage: STAGE,
                    status: status.as_u16(),
                },
            });
        }
        let t: TokenResponse = resp.json().await.map_err(proto(STAGE))?;
        Ok(MsTokens {
            access_token: Secret::new(t.access_token),
            refresh_token: t.refresh_token.map(Secret::new),
        })
    }

    /// Xbox Live → XSTS → Minecraft → entitlement → profile.
    pub async fn minecraft_session(&self, ms: MsTokens) -> Result<MinecraftSession, AuthError> {
        let (xbl, uhs) = self.xbox_live(&ms.access_token).await?;
        let (xsts, xsts_uhs, xuid) = self.xsts(&xbl).await?;
        let uhs = xsts_uhs.unwrap_or(uhs);
        let (mc_token, expires_in) = self.minecraft_login(&uhs, &xsts).await?;
        if !self.owns_game(&mc_token).await? {
            return Err(AuthError::NotOwned);
        }
        let profile = self.profile(&mc_token).await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(MinecraftSession {
            profile,
            access_token: mc_token,
            expires_at_unix: now + expires_in,
            xuid,
            ms_refresh_token: ms.refresh_token,
        })
    }

    async fn xbox_live(&self, ms_access: &Secret) -> Result<(Secret, String), AuthError> {
        const STAGE: &str = "xbox live";
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={}", ms_access.expose()),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });
        let resp = self
            .http
            .post(&self.cfg.xbl_url)
            .json(&body)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net(STAGE))?;
        if !resp.status().is_success() {
            return Err(AuthError::Http {
                stage: STAGE,
                status: resp.status().as_u16(),
            });
        }
        let r: XboxResponse = resp.json().await.map_err(proto(STAGE))?;
        let uhs = r
            .display_claims
            .xui
            .first()
            .and_then(|x| x.uhs.clone())
            .ok_or(AuthError::Protocol {
                stage: STAGE,
                message: "missing user hash".into(),
            })?;
        Ok((Secret::new(r.token), uhs))
    }

    async fn xsts(
        &self,
        xbl: &Secret,
    ) -> Result<(Secret, Option<String>, Option<String>), AuthError> {
        const STAGE: &str = "xsts";
        let body = json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.expose()] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });
        let resp = self
            .http
            .post(&self.cfg.xsts_url)
            .json(&body)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net(STAGE))?;
        let status = resp.status();
        if status.as_u16() == 401 {
            let e: XstsError = resp.json().await.map_err(proto(STAGE))?;
            return Err(match e.xerr {
                2148916233 => AuthError::NoXboxAccount,
                2148916235 => AuthError::XboxRegionUnavailable,
                2148916236 | 2148916237 => AuthError::AdultVerificationRequired,
                2148916238 => AuthError::ChildAccount,
                _ => AuthError::Http {
                    stage: STAGE,
                    status: 401,
                },
            });
        }
        if !status.is_success() {
            return Err(AuthError::Http {
                stage: STAGE,
                status: status.as_u16(),
            });
        }
        let r: XboxResponse = resp.json().await.map_err(proto(STAGE))?;
        let xui = r.display_claims.xui.first();
        Ok((
            Secret::new(r.token),
            xui.and_then(|x| x.uhs.clone()),
            xui.and_then(|x| x.xid.clone()),
        ))
    }

    async fn minecraft_login(&self, uhs: &str, xsts: &Secret) -> Result<(Secret, u64), AuthError> {
        const STAGE: &str = "minecraft login";
        let body = json!({ "identityToken": format!("XBL3.0 x={uhs};{}", xsts.expose()) });
        let resp = self
            .http
            .post(&self.cfg.mc_login_url)
            .json(&body)
            .send()
            .await
            .map_err(net(STAGE))?;
        match resp.status().as_u16() {
            200 => {}
            403 => return Err(AuthError::AppNotApproved),
            429 => return Err(AuthError::RateLimited),
            s => {
                return Err(AuthError::Http {
                    stage: STAGE,
                    status: s,
                });
            }
        }
        let r: McLogin = resp.json().await.map_err(proto(STAGE))?;
        Ok((Secret::new(r.access_token), r.expires_in))
    }

    async fn owns_game(&self, mc: &Secret) -> Result<bool, AuthError> {
        const STAGE: &str = "entitlements";
        let resp = self
            .http
            .get(&self.cfg.mc_entitlements_url)
            .bearer_auth(mc.expose())
            .send()
            .await
            .map_err(net(STAGE))?;
        if !resp.status().is_success() {
            return Err(AuthError::Http {
                stage: STAGE,
                status: resp.status().as_u16(),
            });
        }
        let r: Entitlements = resp.json().await.map_err(proto(STAGE))?;
        Ok(r.items
            .iter()
            .any(|i| i.name == "game_minecraft" || i.name == "product_minecraft"))
    }

    pub async fn profile(&self, mc: &Secret) -> Result<Profile, AuthError> {
        const STAGE: &str = "profile";
        let resp = self
            .http
            .get(&self.cfg.mc_profile_url)
            .bearer_auth(mc.expose())
            .send()
            .await
            .map_err(net(STAGE))?;
        match resp.status().as_u16() {
            200 => resp.json().await.map_err(proto(STAGE)),
            404 => Err(AuthError::NoProfile),
            401 => Err(AuthError::ReauthRequired),
            s => Err(AuthError::Http {
                stage: STAGE,
                status: s,
            }),
        }
    }
}

#[derive(Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    #[serde(default)]
    uhs: Option<String>,
    #[serde(default)]
    xid: Option<String>,
}

#[derive(Deserialize)]
struct XstsError {
    #[serde(rename = "XErr")]
    xerr: u64,
}

#[derive(Deserialize)]
struct McLogin {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Entitlements {
    #[serde(default)]
    items: Vec<EntitlementItem>,
}

#[derive(Deserialize)]
struct EntitlementItem {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_rfc7636_s256() {
        let p = Pkce::generate().unwrap();
        assert_eq!(p.verifier.expose().len(), 43);
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.expose().as_bytes()));
        assert_eq!(p.challenge, expected);
        assert_ne!(Pkce::generate().unwrap().verifier, p.verifier);
    }

    #[test]
    fn authorize_url_contains_required_params() {
        let cfg = AuthConfig::new("cid");
        let p = Pkce::generate().unwrap();
        let u = authorize_url(&cfg, "http://localhost:5000", &p, "st");
        for needle in [
            "client_id=cid",
            "code_challenge_method=S256",
            "state=st",
            "response_type=code",
            "redirect_uri=http%3A%2F%2Flocalhost%3A5000",
        ] {
            assert!(u.contains(needle), "{needle} in {u}");
        }
        assert!(
            !u.contains(p.verifier.expose()),
            "verifier must never be sent in the URL"
        );
    }

    #[test]
    fn request_query_parsing() {
        assert_eq!(
            parse_request_query("GET /?code=a&state=b HTTP/1.1\r\n").as_deref(),
            Some("code=a&state=b")
        );
        assert_eq!(parse_request_query("GET /favicon.ico HTTP/1.1\r\n"), None);
        assert_eq!(parse_request_query("POST /?code=a HTTP/1.1\r\n"), None);
    }
}
