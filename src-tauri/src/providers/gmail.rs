//! Gmail REST API provider (https://developers.google.com/gmail/api).
//!
//! Labels map to folders. Delta sync uses the `history` endpoint keyed by
//! `historyId`; if Google expires the history (404) we fall back to a listing.
//!
//! Gmail meters every call (a message download costs 5 units, 15 000 units
//! per user per minute), so:
//!  * a new folder starts with only its newest `PAGE` messages; older ones are
//!    fetched page by page when the list is scrolled to the end (`fetch_older`);
//!  * calls are paced per mailbox, rate-limit answers retried with backoff;
//!  * messages already stored are not downloaded again, and what was fetched
//!    before a failure is kept.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    api_error, is_rate_limited, FetchResult, FetchedMessage, IncomingProvider, OlderResult, OutgoingProvider, RemoteAction,
    RemoteFolder,
};
use crate::error::{AppError, AppResult};
use crate::models::FolderRole;

pub const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// Rate-limit retries before giving up until the next sync.
const MAX_RETRIES: u32 = 5;
/// Messages per batch: a new folder's first sync, and each "load older".
pub const PAGE: usize = 50;
/// Listing pages scanned per "load older" looking for messages not yet stored.
const MAX_SCAN_PAGES: usize = 10;

/// When each mailbox (keyed by its token) may make its next call: shared by
/// every provider instance, so a sync and a "load older" running together
/// still keep to one pace.
fn next_slot() -> &'static std::sync::Mutex<std::collections::HashMap<String, Instant>> {
    static SLOTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Instant>>> = std::sync::OnceLock::new();
    SLOTS.get_or_init(Default::default)
}

pub struct GmailProvider {
    http: reqwest::Client,
    base: String,
    token: String,
    /// Least time between two calls of a mailbox (10/s ≈ 3 000 units a minute at most).
    pace: Duration,
    /// First wait after a rate-limit answer; doubles on each retry.
    backoff: Duration,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailState {
    pub history_id: String,
    /// Listing page to resume at when loading older messages (none: the first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_page: Option<String>,
    /// Every message of the label has been fetched.
    #[serde(default)]
    pub exhausted: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    email_address: String,
    history_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Label {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct LabelList {
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageList {
    #[serde(default)]
    messages: Vec<MessageRef>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessage {
    id: String,
    raw: String,
    #[serde(default)]
    label_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryList {
    #[serde(default)]
    history: Vec<History>,
    history_id: String,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    #[serde(default)]
    messages_added: Vec<HistoryMessage>,
}

#[derive(Deserialize)]
struct HistoryMessage {
    message: MessageRef,
}

impl GmailProvider {
    pub fn new(token: String) -> Self {
        Self::with_base(token, GMAIL_API_BASE)
    }

    /// Custom base URL — used by tests against a mock server.
    pub fn with_base(token: String, base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into(),
            token,
            pace: Duration::from_millis(100),
            backoff: Duration::from_secs(2),
        }
    }

    /// Tests: no pacing, millisecond backoff.
    pub fn without_pacing(mut self) -> Self {
        self.pace = Duration::ZERO;
        self.backoff = Duration::from_millis(1);
        self
    }

    async fn wait_turn(&self) {
        if self.pace.is_zero() {
            return;
        }
        // Reserve the next slot, then sleep until it outside the lock.
        let slot = {
            let mut slots = next_slot().lock().unwrap();
            let now = Instant::now();
            let slot = slots.get(&self.token).copied().filter(|t| *t > now).unwrap_or(now);
            slots.insert(self.token.clone(), slot + self.pace);
            slots.retain(|_, t| *t + Duration::from_secs(60) > now);
            slot
        };
        tokio::time::sleep_until(slot.into()).await;
    }

    /// Sends a request built by `build`, paced, retrying rate-limit answers
    /// (honouring `Retry-After`) with exponential backoff.
    async fn call(&self, build: impl Fn() -> reqwest::RequestBuilder) -> AppResult<reqwest::Response> {
        let mut attempt = 0;
        loop {
            self.wait_turn().await;
            let resp = build().bearer_auth(&self.token).send().await?;
            let status = resp.status();
            if status.is_success() {
                return Ok(resp);
            }
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let path = resp.url().path().to_string();
            let body = resp.text().await.unwrap_or_default();
            if is_rate_limited(status.as_u16(), &body) && attempt < MAX_RETRIES {
                let wait = retry_after
                    .map(Duration::from_secs)
                    .unwrap_or(self.backoff * 2u32.pow(attempt))
                    .min(Duration::from_secs(60));
                log::info!("Gmail rate limit on {path}; retrying in {wait:?}");
                tokio::time::sleep(wait).await;
                attempt += 1;
                continue;
            }
            return Err(api_error(status.as_u16(), &path, &body));
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> AppResult<T> {
        let url = format!("{}{path}", self.base);
        Ok(self.call(|| self.http.get(&url).query(query)).await?.json().await?)
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> AppResult<()> {
        let url = format!("{}{path}", self.base);
        self.call(|| self.http.post(&url).json(&body)).await?;
        Ok(())
    }

    pub async fn profile_email(&self) -> AppResult<String> {
        Ok(self.get::<Profile>("/profile", &[]).await?.email_address)
    }

    async fn fetch_raw(&self, id: &str) -> AppResult<FetchedMessage> {
        let m: RawMessage = self.get(&format!("/messages/{id}"), &[("format", "raw".into())]).await?;
        let raw = URL_SAFE_NO_PAD
            .decode(m.raw.trim_end_matches('=').as_bytes())
            .map_err(|e| AppError::Protocol(format!("bad base64 in Gmail message: {e}")))?;
        Ok(FetchedMessage {
            remote_id: m.id,
            raw,
            is_read: !m.label_ids.iter().any(|l| l == "UNREAD"),
            is_flagged: m.label_ids.iter().any(|l| l == "STARRED"),
        })
    }

    /// One listing page of a label, newest first: ids and the next page token.
    async fn list_page(&self, label: &str, page: Option<&str>, size: usize) -> AppResult<(Vec<String>, Option<String>)> {
        let mut q = vec![("labelIds", label.to_string()), ("maxResults", size.to_string())];
        if let Some(p) = page {
            q.push(("pageToken", p.to_string()));
        }
        let list: MessageList = self.get("/messages", &q).await?;
        Ok((list.messages.into_iter().map(|m| m.id).collect(), list.next_page_token))
    }

    /// Downloads `ids`; after a failure keeps what it has (`Ok(false)` = incomplete).
    async fn download(&self, label: &str, ids: Vec<String>, into: &mut Vec<FetchedMessage>) -> AppResult<bool> {
        for id in ids {
            match self.fetch_raw(&id).await {
                Ok(m) => into.push(m),
                // Message deleted between listing and fetch.
                Err(AppError::NotFound(_)) => continue,
                Err(e) if !into.is_empty() && !matches!(e, AppError::Auth(_)) => {
                    log::warn!("Gmail {label}: stopped after {} messages: {e}", into.len());
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// Message ids added to `label` since `start`, plus the new history id.
    async fn history_since(&self, label: &str, start: &str) -> AppResult<(Vec<String>, String)> {
        let mut ids = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut q = vec![
                ("startHistoryId", start.to_string()),
                ("labelId", label.to_string()),
                ("historyTypes", "messageAdded".into()),
            ];
            if let Some(p) = &page {
                q.push(("pageToken", p.clone()));
            }
            let h: HistoryList = self.get("/history", &q).await?;
            for entry in h.history {
                ids.extend(entry.messages_added.into_iter().map(|m| m.message.id));
            }
            page = h.next_page_token;
            if page.is_none() {
                ids.dedup();
                return Ok((ids, h.history_id));
            }
        }
    }
}

fn system_role(id: &str) -> Option<(FolderRole, &'static str)> {
    Some(match id {
        "INBOX" => (FolderRole::Inbox, "Inbox"),
        "SENT" => (FolderRole::Sent, "Sent"),
        "DRAFT" => (FolderRole::Drafts, "Drafts"),
        "SPAM" => (FolderRole::Spam, "Spam"),
        "TRASH" => (FolderRole::Trash, "Trash"),
        _ => return None,
    })
}

#[async_trait]
impl IncomingProvider for GmailProvider {
    async fn test(&self) -> AppResult<()> {
        self.profile_email().await.map(|_| ())
    }

    async fn list_folders(&self) -> AppResult<Vec<RemoteFolder>> {
        let labels: LabelList = self.get("/labels", &[]).await?;
        Ok(labels
            .labels
            .into_iter()
            .filter_map(|l| {
                if let Some((role, name)) = system_role(&l.id) {
                    return Some(RemoteFolder { name: name.into(), path: l.id, role });
                }
                (l.kind.as_deref() == Some("user")).then_some(RemoteFolder { name: l.name, path: l.id, role: FolderRole::Custom })
            })
            .collect())
    }

    async fn fetch(&self, folder: &RemoteFolder, state: Option<&str>) -> AppResult<FetchResult> {
        self.fetch_new(folder, state, &HashSet::new()).await
    }

    async fn fetch_new(&self, folder: &RemoteFolder, state: Option<&str>, known: &HashSet<String>) -> AppResult<FetchResult> {
        let state: Option<GmailState> = state.and_then(|s| serde_json::from_str(s).ok());
        let newest_page = || async {
            // Take the cursor before listing so nothing slips through.
            let hid = self.get::<Profile>("/profile", &[]).await?.history_id;
            let (ids, next) = self.list_page(&folder.path, None, PAGE).await?;
            let exhausted = next.is_none();
            AppResult::Ok((ids, GmailState { history_id: hid, older_page: next, exhausted }))
        };
        let (ids, new_state) = match state {
            Some(st) => match self.history_since(&folder.path, &st.history_id).await {
                Ok((ids, history_id)) => (ids, GmailState { history_id, ..st }),
                // History expired → start again from the newest page.
                Err(AppError::NotFound(_)) => newest_page().await?,
                Err(e) => return Err(e),
            },
            None => newest_page().await?,
        };

        let mut messages = Vec::new();
        let complete = self.download(&folder.path, ids.into_iter().filter(|id| !known.contains(id)).collect(), &mut messages).await?;
        // Incomplete: the cursor stays put, so the next sync fetches only what is missing.
        let new_state = if complete { Some(serde_json::to_string(&new_state)?) } else { None };
        Ok(FetchResult { messages, new_state })
    }

    async fn fetch_older(
        &self,
        folder: &RemoteFolder,
        state: Option<&str>,
        known: &HashSet<String>,
        count: usize,
    ) -> AppResult<OlderResult> {
        let Some(mut st) = state.and_then(|s| serde_json::from_str::<GmailState>(s).ok()) else {
            // Not synced yet: the regular sync comes first.
            return Ok(OlderResult::default());
        };
        if st.exhausted {
            return Ok(OlderResult { exhausted: true, ..Default::default() });
        }
        // Walk the listing from where the last batch stopped, collecting ids not
        // stored yet. Listing is cheap; downloads are what the quota pays for.
        let mut wanted = Vec::new();
        let mut page = st.older_page.clone();
        let mut exhausted = false;
        for _ in 0..MAX_SCAN_PAGES {
            let (ids, next) = self.list_page(&folder.path, page.as_deref(), 100).await?;
            for id in ids {
                if !known.contains(&id) && !wanted.contains(&id) {
                    wanted.push(id);
                }
            }
            if wanted.len() >= count {
                // Unfetched ids may remain on this page: resume here next time.
                break;
            }
            match next {
                Some(n) => page = Some(n),
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        wanted.truncate(count);

        let mut messages = Vec::new();
        let complete = self.download(&folder.path, wanted, &mut messages).await?;
        st.older_page = page;
        st.exhausted = exhausted && complete;
        Ok(OlderResult { messages, new_state: Some(serde_json::to_string(&st)?), exhausted: st.exhausted })
    }

    async fn apply(&self, folder_path: &str, remote_id: &str, action: &RemoteAction) -> AppResult<()> {
        let modify = |add: Vec<&str>, remove: Vec<&str>| json!({ "addLabelIds": add, "removeLabelIds": remove });
        let path = format!("/messages/{remote_id}/modify");
        match action {
            RemoteAction::MarkRead(true) => self.post(&path, modify(vec![], vec!["UNREAD"])).await,
            RemoteAction::MarkRead(false) => self.post(&path, modify(vec!["UNREAD"], vec![])).await,
            RemoteAction::Flag(true) => self.post(&path, modify(vec!["STARRED"], vec![])).await,
            RemoteAction::Flag(false) => self.post(&path, modify(vec![], vec!["STARRED"])).await,
            RemoteAction::Move { to_path } => self.post(&path, modify(vec![to_path], vec![folder_path])).await,
            RemoteAction::Archive { .. } => self.post(&path, modify(vec![], vec!["INBOX"])).await,
            RemoteAction::Trash { .. } => self.post(&format!("/messages/{remote_id}/trash"), json!({})).await,
            RemoteAction::Delete => {
                let url = format!("{}/messages/{remote_id}", self.base);
                self.call(|| self.http.delete(&url)).await?;
                Ok(())
            }
        }
    }
}

#[async_trait]
impl OutgoingProvider for GmailProvider {
    async fn test(&self) -> AppResult<()> {
        self.profile_email().await.map(|_| ())
    }

    async fn send(&self, message: &lettre::Message) -> AppResult<()> {
        let raw = URL_SAFE_NO_PAD.encode(message.formatted());
        self.post("/messages/send", json!({ "raw": raw })).await
    }
}
