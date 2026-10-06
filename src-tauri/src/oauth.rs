//! OAuth 2.0 Authorization Code + PKCE for Google and Microsoft.
//!
//! Desktop flow (RFC 8252): a one-shot HTTP listener on `127.0.0.1:<random>`
//! receives the redirect, the system browser performs the login. Only the
//! refresh token is persisted (OS keychain); access tokens live in memory and
//! are refreshed transparently a minute before they expire.
//!
//! The OAuth app (client id, and for Google the desktop client secret) is
//! looked up in this order:
//!  1. Settings → Sign-in apps: saved in the OS keychain, never in `mail.db`
//!     and never sent back to the UI;
//!  2. the environment at runtime;
//!  3. values baked in at compile time:
//!     `TRAJECTORY_GOOGLE_CLIENT_ID` / `TRAJECTORY_GOOGLE_CLIENT_SECRET`,
//!     `TRAJECTORY_MS_CLIENT_ID`
//!     (the pre-rename `SINGULARITY_*` names are still read as a fallback).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use std::time::Duration;

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet, PkceCodeChallenge,
    RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};
use crate::models::AccountKind;
use crate::providers::{gmail::GmailProvider, graph::GraphProvider};
use crate::secrets::{SecretStore, KEY_REFRESH_TOKEN};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Keychain "account" under which the OAuth apps saved in Settings live.
pub const OAUTH_APP_OWNER: &str = "oauth-app";

/// Where an OAuth app's client id comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppSource {
    /// Saved in Settings (OS keychain).
    Settings,
    /// Environment variable at runtime.
    Environment,
    /// Baked into the binary at build time.
    Build,
}

/// The OAuth client used to sign in.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthApp {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub source: AppSource,
}

impl std::fmt::Debug for OAuthApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthApp")
            .field("client_id", &self.client_id)
            .field("has_secret", &self.client_secret.is_some())
            .field("source", &self.source)
            .finish()
    }
}

/// What the UI is told about an OAuth app: never the secret itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAppInfo {
    /// "google" or "microsoft".
    pub provider: String,
    pub client_id: Option<String>,
    /// Masked secret, e.g. `GOCSPX-••••••`; none when there is no secret.
    pub secret_hint: Option<String>,
    pub source: Option<AppSource>,
}

/// `GOCSPX-abc…` → `GOCSPX-••••••`: enough to recognise it, nothing to reuse.
fn mask(secret: &str) -> String {
    match secret.find('-').filter(|&i| i <= 8 && i + 1 < secret.len()) {
        Some(i) => format!("{}••••••", &secret[..=i]),
        None => "••••••".into(),
    }
}

type OAuthClient = BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    Google,
    Microsoft,
}

impl OAuthProvider {
    pub fn for_kind(kind: AccountKind) -> AppResult<Self> {
        match kind {
            AccountKind::Gmail => Ok(OAuthProvider::Google),
            AccountKind::Outlook => Ok(OAuthProvider::Microsoft),
            _ => Err(AppError::Invalid(format!("{} accounts do not use OAuth", kind.as_str()))),
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "google" => Ok(OAuthProvider::Google),
            "microsoft" => Ok(OAuthProvider::Microsoft),
            _ => Err(AppError::Invalid(format!("unknown OAuth provider '{s}'"))),
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            OAuthProvider::Google => "google",
            OAuthProvider::Microsoft => "microsoft",
        }
    }

    /// Keychain keys of the client id and secret saved in Settings.
    fn keys(&self) -> (&'static str, &'static str) {
        match self {
            OAuthProvider::Google => ("google_client_id", "google_client_secret"),
            OAuthProvider::Microsoft => ("ms_client_id", "ms_client_secret"),
        }
    }

    /// Whether the provider's desktop client needs its (non-confidential) secret.
    pub fn needs_secret(&self) -> bool {
        *self == OAuthProvider::Google
    }

    pub fn kind(&self) -> AccountKind {
        match self {
            OAuthProvider::Google => AccountKind::Gmail,
            OAuthProvider::Microsoft => AccountKind::Outlook,
        }
    }

    pub fn auth_url(&self) -> &'static str {
        match self {
            OAuthProvider::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            OAuthProvider::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        }
    }

    pub fn token_url(&self) -> &'static str {
        match self {
            OAuthProvider::Google => "https://oauth2.googleapis.com/token",
            OAuthProvider::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        }
    }

    pub fn scopes(&self) -> &'static [&'static str] {
        match self {
            OAuthProvider::Google => &[
                "openid",
                "email",
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.send",
            ],
            OAuthProvider::Microsoft => &["offline_access", "User.Read", "Mail.ReadWrite", "Mail.Send"],
        }
    }

    /// The app from the environment or the build, if any.
    fn env_app(&self) -> Option<OAuthApp> {
        let (env, legacy, baked) = match self {
            OAuthProvider::Google => (
                "TRAJECTORY_GOOGLE_CLIENT_ID",
                "SINGULARITY_GOOGLE_CLIENT_ID",
                option_env!("TRAJECTORY_GOOGLE_CLIENT_ID").or(option_env!("SINGULARITY_GOOGLE_CLIENT_ID")),
            ),
            OAuthProvider::Microsoft => (
                "TRAJECTORY_MS_CLIENT_ID",
                "SINGULARITY_MS_CLIENT_ID",
                option_env!("TRAJECTORY_MS_CLIENT_ID").or(option_env!("SINGULARITY_MS_CLIENT_ID")),
            ),
        };
        let nonempty = |s: String| (!s.trim().is_empty()).then(|| s.trim().to_string());
        let runtime = std::env::var(env).or_else(|_| std::env::var(legacy)).ok().and_then(nonempty);
        let (client_id, source) = match runtime {
            Some(id) => (id, AppSource::Environment),
            None => (baked.map(str::to_string).and_then(nonempty)?, AppSource::Build),
        };
        let client_secret = match self {
            OAuthProvider::Google => match source {
                AppSource::Environment => std::env::var("TRAJECTORY_GOOGLE_CLIENT_SECRET")
                    .or_else(|_| std::env::var("SINGULARITY_GOOGLE_CLIENT_SECRET"))
                    .ok(),
                _ => option_env!("TRAJECTORY_GOOGLE_CLIENT_SECRET")
                    .or(option_env!("SINGULARITY_GOOGLE_CLIENT_SECRET"))
                    .map(str::to_string),
            }
            .and_then(nonempty),
            OAuthProvider::Microsoft => None,
        };
        Some(OAuthApp { client_id, client_secret, source })
    }

    /// The app saved in Settings, if any.
    fn saved_app(&self, secrets: &dyn SecretStore) -> AppResult<Option<OAuthApp>> {
        let (id_key, secret_key) = self.keys();
        let Some(client_id) = secrets.get(OAUTH_APP_OWNER, id_key)? else { return Ok(None) };
        Ok(Some(OAuthApp { client_id, client_secret: secrets.get(OAUTH_APP_OWNER, secret_key)?, source: AppSource::Settings }))
    }

    /// The app to sign in with: Settings, then the environment, then the build.
    pub fn app(&self, secrets: &dyn SecretStore) -> AppResult<OAuthApp> {
        if let Some(app) = self.saved_app(secrets)? {
            return Ok(app);
        }
        self.env_app().ok_or_else(|| {
            let name = match self {
                OAuthProvider::Google => "Google",
                OAuthProvider::Microsoft => "Microsoft",
            };
            AppError::Invalid(format!("{name} sign-in is not set up: add your client ID in Settings → Sign-in apps"))
        })
    }

    /// What Settings shows about the app in use.
    pub fn info(&self, secrets: &dyn SecretStore) -> AppResult<OAuthAppInfo> {
        let app = match self.saved_app(secrets)? {
            Some(app) => Some(app),
            None => self.env_app(),
        };
        Ok(OAuthAppInfo {
            provider: self.id().into(),
            client_id: app.as_ref().map(|a| a.client_id.clone()),
            secret_hint: app.as_ref().and_then(|a| a.client_secret.as_deref()).map(mask),
            source: app.map(|a| a.source),
        })
    }

    /// Saves the app in the keychain. `client_secret: None` keeps the saved one.
    pub fn save_app(&self, secrets: &dyn SecretStore, client_id: &str, client_secret: Option<&str>) -> AppResult<()> {
        let client_id = client_id.trim();
        let valid_id = match self {
            // 1234567890-abc123.apps.googleusercontent.com
            OAuthProvider::Google => client_id
                .strip_suffix(".apps.googleusercontent.com")
                .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')),
            // Application (client) ID: a GUID.
            OAuthProvider::Microsoft => {
                let parts: Vec<&str> = client_id.split('-').collect();
                parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
            }
        };
        if !valid_id {
            return Err(AppError::Invalid(match self {
                OAuthProvider::Google => "a Google client ID ends with .apps.googleusercontent.com".into(),
                OAuthProvider::Microsoft => "a Microsoft client ID looks like 00000000-0000-0000-0000-000000000000".into(),
            }));
        }
        let (id_key, secret_key) = self.keys();
        let new_secret = client_secret.map(str::trim).filter(|s| !s.is_empty());
        if let Some(s) = new_secret {
            if s.len() > 256 || s.chars().any(char::is_whitespace) {
                return Err(AppError::Invalid("the client secret has an unexpected format".into()));
            }
        }
        if self.needs_secret() && new_secret.is_none() && secrets.get(OAUTH_APP_OWNER, secret_key)?.is_none() {
            return Err(AppError::Invalid("a Google desktop client also needs its client secret".into()));
        }
        secrets.set(OAUTH_APP_OWNER, id_key, client_id)?;
        if let Some(s) = new_secret {
            secrets.set(OAUTH_APP_OWNER, secret_key, s)?;
        }
        Ok(())
    }

    /// Forgets the app saved in Settings (the environment/build one applies again).
    pub fn clear_app(&self, secrets: &dyn SecretStore) -> AppResult<()> {
        let (id_key, secret_key) = self.keys();
        secrets.delete(OAUTH_APP_OWNER, id_key)?;
        secrets.delete(OAUTH_APP_OWNER, secret_key)
    }

    fn client(&self, app: &OAuthApp, redirect: &str) -> AppResult<OAuthClient> {
        let url_err = |e: url::ParseError| AppError::Invalid(e.to_string());
        let mut client = BasicClient::new(ClientId::new(app.client_id.clone()))
            .set_auth_uri(AuthUrl::new(self.auth_url().into()).map_err(url_err)?)
            .set_token_uri(TokenUrl::new(self.token_url().into()).map_err(url_err)?)
            .set_redirect_uri(RedirectUrl::new(redirect.into()).map_err(url_err)?);
        if let Some(secret) = app.client_secret.clone().filter(|s| !s.is_empty()) {
            client = client.set_client_secret(ClientSecret::new(secret));
        }
        Ok(client)
    }
}

#[derive(Clone)]
pub struct OAuthGrant {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

impl std::fmt::Debug for OAuthGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthGrant").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

fn http_client() -> AppResult<reqwest::Client> {
    // Redirects must not be followed during token exchange (SSRF hardening).
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

fn grant_from<T: TokenResponse>(t: &T) -> OAuthGrant {
    OAuthGrant {
        access_token: t.access_token().secret().clone(),
        refresh_token: t.refresh_token().map(|r| r.secret().clone()),
        expires_at: chrono::Utc::now().timestamp() + t.expires_in().map(|d| d.as_secs() as i64).unwrap_or(3600),
    }
}

/// Parses the loopback redirect request line: `GET /callback?code=..&state=.. HTTP/1.1`.
pub fn parse_callback(request_line: &str) -> AppResult<Option<(String, String)>> {
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::Protocol("malformed redirect request".into()))?;
    let url = url::Url::parse(&format!("http://localhost{target}"))
        .map_err(|e| AppError::Protocol(format!("malformed redirect: {e}")))?;
    if url.path() != "/callback" {
        return Ok(None);
    }
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(err) = params.get("error") {
        let desc = params.get("error_description").cloned().unwrap_or_default();
        return Err(AppError::Auth(format!("authorization denied: {err} {desc}").trim().to_string()));
    }
    match (params.get("code"), params.get("state")) {
        (Some(c), Some(s)) => Ok(Some((c.clone(), s.clone()))),
        _ => Err(AppError::Protocol("redirect is missing code/state".into())),
    }
}

const DONE_PAGE: &str = "<!doctype html><html><body style=\"background:#0b0c0e;color:#e5e7eb;font-family:system-ui;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h2>Signed in to Trajectory</h2><p style=\"color:#9ca3af\">You can close this tab and return to the app.</p></div></body></html>";

/// Waits for the browser to hit `/callback`, answering other requests with 404.
async fn wait_for_callback(listener: TcpListener) -> AppResult<(String, String)> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let line = req.lines().next().unwrap_or_default().to_string();
        match parse_callback(&line) {
            Ok(Some(result)) => {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{DONE_PAGE}",
                    DONE_PAGE.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                return Ok(result);
            }
            Ok(None) => {
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
            Err(e) => {
                let _ = sock
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                return Err(e);
            }
        }
    }
}

/// Runs the full interactive authorization. `open_browser` receives the
/// authorization URL. Returns the grant and the mailbox address.
pub async fn authorize<F>(provider: OAuthProvider, app: &OAuthApp, open_browser: F) -> AppResult<(OAuthGrant, String)>
where
    F: FnOnce(&str) -> AppResult<()>,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let client = provider.client(app, &redirect)?;

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut req = client
        .authorize_url(CsrfToken::new_random)
        .add_scopes(provider.scopes().iter().map(|s| Scope::new(s.to_string())))
        .set_pkce_challenge(challenge);
    if provider == OAuthProvider::Google {
        req = req.add_extra_param("access_type", "offline").add_extra_param("prompt", "consent");
    } else {
        req = req.add_extra_param("prompt", "select_account");
    }
    let (auth_url, csrf) = req.url();
    open_browser(auth_url.as_str())?;

    let (code, state) = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(listener))
        .await
        .map_err(|_| AppError::Auth("timed out waiting for browser sign-in".into()))??;
    if state != *csrf.secret() {
        return Err(AppError::Auth("OAuth state mismatch (possible CSRF)".into()));
    }

    let token = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(verifier)
        .request_async(&http_client()?)
        .await
        .map_err(|e| AppError::Auth(format!("token exchange failed: {e}")))?;
    let grant = grant_from(&token);
    if grant.refresh_token.is_none() {
        return Err(AppError::Auth("provider did not return a refresh token".into()));
    }

    let email = match provider {
        OAuthProvider::Google => GmailProvider::new(grant.access_token.clone()).profile_email().await?,
        OAuthProvider::Microsoft => GraphProvider::new(grant.access_token.clone()).profile_email().await?,
    };
    Ok((grant, email))
}

pub async fn refresh(provider: OAuthProvider, app: &OAuthApp, refresh_token: &str) -> AppResult<OAuthGrant> {
    // The redirect URI is irrelevant for refresh but required by the client type.
    let client = provider.client(app, "http://127.0.0.1/callback")?;
    let token = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
        .request_async(&http_client()?)
        .await
        .map_err(|e| AppError::Auth(format!("token refresh failed: {e}")))?;
    Ok(grant_from(&token))
}

/// In-memory access token cache keyed by account id.
#[derive(Default)]
pub struct TokenCache {
    tokens: Mutex<HashMap<String, (String, i64)>>,
}

impl TokenCache {
    pub fn put(&self, account_id: &str, grant: &OAuthGrant) {
        self.tokens
            .lock()
            .unwrap()
            .insert(account_id.to_string(), (grant.access_token.clone(), grant.expires_at));
    }

    pub fn forget(&self, account_id: &str) {
        self.tokens.lock().unwrap().remove(account_id);
    }

    fn cached(&self, account_id: &str) -> Option<String> {
        let now = chrono::Utc::now().timestamp();
        self.tokens
            .lock()
            .unwrap()
            .get(account_id)
            .filter(|(_, exp)| *exp - 60 > now)
            .map(|(t, _)| t.clone())
    }

    /// Returns a valid access token, refreshing (and rotating the stored
    /// refresh token, as Microsoft does) when needed.
    pub async fn access_token(
        &self,
        account_id: &str,
        kind: AccountKind,
        secrets: &dyn SecretStore,
    ) -> AppResult<String> {
        if let Some(t) = self.cached(account_id) {
            return Ok(t);
        }
        let provider = OAuthProvider::for_kind(kind)?;
        let refresh_token = secrets.require(account_id, KEY_REFRESH_TOKEN)?;
        let grant = refresh(provider, &provider.app(secrets)?, &refresh_token).await?;
        if let Some(rt) = grant.refresh_token.as_deref().filter(|rt| *rt != refresh_token) {
            secrets.set(account_id, KEY_REFRESH_TOKEN, rt)?;
        }
        self.put(account_id, &grant);
        Ok(grant.access_token)
    }
}
