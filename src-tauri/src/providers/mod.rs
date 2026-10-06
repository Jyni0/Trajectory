//! Protocol adapters. Every account is backed by one incoming provider
//! (IMAP / POP3 / Gmail API / Microsoft Graph) and one outgoing provider
//! (SMTP / Gmail API / Microsoft Graph).

pub mod gmail;
pub mod graph;
pub mod imap;
pub mod pop3;
pub mod smtp;

use std::collections::HashSet;

use async_trait::async_trait;

use crate::error::{AppError, AppResult};
use crate::models::{Account, AccountKind, FolderRole};

/// `error_for_status` that keeps the API's own explanation: Google and
/// Microsoft both answer `{"error": {"message": "…"}}`, which says *why*
/// (Gmail API disabled in the project, a scope not granted, …).
#[async_trait]
pub(crate) trait CheckStatus: Sized {
    async fn checked(self) -> AppResult<reqwest::Response>;
}

#[async_trait]
impl CheckStatus for reqwest::Response {
    async fn checked(self) -> AppResult<reqwest::Response> {
        let status = self.status();
        if status.is_success() {
            return Ok(self);
        }
        let path = self.url().path().to_string();
        let body = self.text().await.unwrap_or_default();
        Err(api_error(status.as_u16(), &path, &body))
    }
}

/// Whether a failed call means "slow down" rather than "not allowed": HTTP
/// 429, or Google's 403 with a rate-limit / per-minute quota reason.
pub(crate) fn is_rate_limited(status: u16, body: &str) -> bool {
    status == 429
        || (status == 403
            && ["rateLimitExceeded", "userRateLimitExceeded", "RATE_LIMIT_EXCEEDED", "Quota exceeded"]
                .iter()
                .any(|m| body.contains(m)))
}

/// The error for a failed API call, with the reason the server gave.
pub(crate) fn api_error(status: u16, path: &str, body: &str) -> AppError {
    let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let reason = json
        .as_ref()
        .and_then(|v| v["error"]["message"].as_str().or(v["error_description"].as_str()).or(v["error"].as_str()))
        .map(|m| m.chars().take(400).collect::<String>())
        .unwrap_or_else(|| reqwest::StatusCode::from_u16(status).ok().and_then(|s| s.canonical_reason()).unwrap_or("error").to_string());
    let hint = if reason.contains("insufficient authentication scopes") {
        " — sign in again and allow every permission on Google's consent screen"
    } else {
        ""
    };
    let msg = format!("{reason}{hint} ({status} {path})");
    // A rate limit is not a credentials problem: the next sync simply carries on.
    if is_rate_limited(status, body) {
        return AppError::Network(format!("rate limited, will continue on the next sync: {msg}"));
    }
    match status {
        401 | 403 => AppError::Auth(msg),
        404 => AppError::NotFound(msg),
        _ => AppError::Network(msg),
    }
}

/// Initial sync depth per folder; later syncs are incremental.
pub const INITIAL_SYNC_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFolder {
    pub name: String,
    pub path: String,
    pub role: FolderRole,
}

#[derive(Debug, Clone)]
pub struct FetchedMessage {
    pub remote_id: String,
    pub raw: Vec<u8>,
    pub is_read: bool,
    pub is_flagged: bool,
}

#[derive(Debug, Default)]
pub struct FetchResult {
    pub messages: Vec<FetchedMessage>,
    /// Opaque provider cursor persisted in `sync_state` (UIDVALIDITY/last UID,
    /// seen UIDLs, Gmail historyId, Graph deltaLink).
    pub new_state: Option<String>,
}

/// A batch of older messages, fetched when the list is scrolled to its end.
#[derive(Debug, Default)]
pub struct OlderResult {
    pub messages: Vec<FetchedMessage>,
    /// Cursor to persist (it remembers how far back the folder was read).
    pub new_state: Option<String>,
    /// Nothing older is left on the server.
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoteAction {
    MarkRead(bool),
    Flag(bool),
    Move { to_path: String },
    /// Move to the trash folder (or the provider's trash semantics).
    Trash { trash_path: Option<String> },
    /// Archive (Gmail: remove INBOX label).
    Archive { archive_path: Option<String> },
    /// Permanent delete.
    Delete,
}

#[async_trait]
pub trait IncomingProvider: Send + Sync {
    async fn test(&self) -> AppResult<()>;
    async fn list_folders(&self) -> AppResult<Vec<RemoteFolder>>;
    async fn fetch(&self, folder: &RemoteFolder, state: Option<&str>) -> AppResult<FetchResult>;
    /// Like `fetch`, but may skip downloading messages the folder already has
    /// (`known` remote ids). Providers that pay per message (Gmail) use it.
    async fn fetch_new(&self, folder: &RemoteFolder, state: Option<&str>, _known: &HashSet<String>) -> AppResult<FetchResult> {
        self.fetch(folder, state).await
    }
    async fn apply(&self, folder_path: &str, remote_id: &str, action: &RemoteAction) -> AppResult<()>;
    /// Up to `count` messages older than what the folder holds, on demand.
    /// Providers that cannot page back report `exhausted` straight away.
    async fn fetch_older(
        &self,
        _folder: &RemoteFolder,
        _state: Option<&str>,
        _known: &HashSet<String>,
        _count: usize,
    ) -> AppResult<OlderResult> {
        Ok(OlderResult { exhausted: true, ..Default::default() })
    }
    /// Stores a sent copy on the server (IMAP only — APIs do this themselves).
    async fn append(&self, _folder_path: &str, _raw: &[u8]) -> AppResult<()> {
        Ok(())
    }
}

#[async_trait]
pub trait OutgoingProvider: Send + Sync {
    async fn test(&self) -> AppResult<()>;
    async fn send(&self, message: &lettre::Message) -> AppResult<()>;
}

/// Credentials resolved for a single operation.
#[derive(Clone)]
pub enum Credentials {
    Password(String),
    OAuth { access_token: String },
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credentials::Password(_) => f.write_str("Password(***)"),
            Credentials::OAuth { .. } => f.write_str("OAuth(***)"),
        }
    }
}

fn username(account: &Account) -> String {
    account.username.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| account.email.clone())
}

fn password(creds: &Credentials) -> AppResult<String> {
    match creds {
        Credentials::Password(p) => Ok(p.clone()),
        Credentials::OAuth { .. } => Err(AppError::Invalid("account expects a password".into())),
    }
}

fn token(creds: &Credentials) -> AppResult<String> {
    match creds {
        Credentials::OAuth { access_token } => Ok(access_token.clone()),
        Credentials::Password(_) => Err(AppError::Invalid("account expects an OAuth token".into())),
    }
}

pub fn incoming_for(account: &Account, creds: &Credentials) -> AppResult<Box<dyn IncomingProvider>> {
    Ok(match account.kind {
        AccountKind::Gmail => Box::new(gmail::GmailProvider::new(token(creds)?)),
        AccountKind::Outlook => Box::new(graph::GraphProvider::new(token(creds)?)),
        AccountKind::Imap => Box::new(imap::ImapProvider::new(
            account.incoming.clone().ok_or_else(|| AppError::Invalid("missing IMAP settings".into()))?,
            username(account),
            password(creds)?,
        )),
        AccountKind::Pop3 => Box::new(pop3::Pop3Provider::new(
            account.incoming.clone().ok_or_else(|| AppError::Invalid("missing POP3 settings".into()))?,
            username(account),
            password(creds)?,
            account.pop3_download_days,
        )),
    })
}

pub fn outgoing_for(account: &Account, creds: &Credentials) -> AppResult<Box<dyn OutgoingProvider>> {
    Ok(match account.kind {
        AccountKind::Gmail => Box::new(gmail::GmailProvider::new(token(creds)?)),
        AccountKind::Outlook => Box::new(graph::GraphProvider::new(token(creds)?)),
        AccountKind::Imap | AccountKind::Pop3 => Box::new(smtp::SmtpProvider::new(
            account.outgoing.clone().ok_or_else(|| AppError::Invalid("missing SMTP settings".into()))?,
            username(account),
            password(creds)?,
        )),
    })
}

/// Maps well-known mailbox names (English + Russian + common vendor names)
/// to roles when the server does not advertise SPECIAL-USE attributes.
pub fn role_from_name(name: &str) -> FolderRole {
    let n = name.rsplit(['/', '.']).next().unwrap_or(name).trim().to_lowercase();
    match n.as_str() {
        "inbox" | "входящие" => FolderRole::Inbox,
        "sent" | "sent items" | "sent mail" | "sent messages" | "отправленные" => FolderRole::Sent,
        "drafts" | "draft" | "черновики" => FolderRole::Drafts,
        "spam" | "junk" | "junk e-mail" | "junk email" | "bulk mail" | "спам" => FolderRole::Spam,
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" | "корзина" | "удаленные" | "удалённые" => {
            FolderRole::Trash
        }
        "archive" | "archives" | "all mail" | "архив" => FolderRole::Archive,
        _ => FolderRole::Custom,
    }
}
