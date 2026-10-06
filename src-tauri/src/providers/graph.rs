//! Microsoft Graph provider for Outlook.com / Office 365 mailboxes.
//!
//! Folder delta queries (`/mailFolders/{id}/messages/delta`) drive incremental
//! sync; the opaque `@odata.deltaLink` is the persisted cursor. Messages are
//! downloaded as MIME (`/messages/{id}/$value`) so they share the same parser
//! as IMAP/POP3.

use super::CheckStatus;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{FetchResult, FetchedMessage, IncomingProvider, OutgoingProvider, RemoteAction, RemoteFolder, INITIAL_SYNC_LIMIT};
use crate::error::{AppError, AppResult};
use crate::models::FolderRole;

pub const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0/me";

const WELL_KNOWN: &[(&str, FolderRole)] = &[
    ("inbox", FolderRole::Inbox),
    ("sentitems", FolderRole::Sent),
    ("drafts", FolderRole::Drafts),
    ("junkemail", FolderRole::Spam),
    ("deleteditems", FolderRole::Trash),
    ("archive", FolderRole::Archive),
];

pub struct GraphProvider {
    http: reqwest::Client,
    base: String,
    token: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphState {
    pub delta_link: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MailFolder {
    id: String,
    display_name: String,
}

#[derive(Deserialize)]
struct FolderList {
    #[serde(default)]
    value: Vec<MailFolder>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct DeltaPage {
    #[serde(default)]
    value: Vec<Value>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

impl GraphProvider {
    pub fn new(token: String) -> Self {
        Self::with_base(token, GRAPH_API_BASE)
    }

    pub fn with_base(token: String, base: impl Into<String>) -> Self {
        Self { http: reqwest::Client::new(), base: base.into(), token }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{path}", self.base)
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> AppResult<T> {
        Ok(self
            .http
            .get(self.url(path))
            .bearer_auth(&self.token)
            .header("Prefer", "odata.maxpagesize=50")
            .send()
            .await?
            .checked().await?
            .json()
            .await?)
    }

    async fn send_json(&self, method: reqwest::Method, path: &str, body: Value) -> AppResult<()> {
        self.http
            .request(method, self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?
            .checked().await?;
        Ok(())
    }

    pub async fn profile_email(&self) -> AppResult<String> {
        let me: Value = self.get_json("").await?;
        me.get("mail")
            .and_then(Value::as_str)
            .or_else(|| me.get("userPrincipalName").and_then(Value::as_str))
            .map(str::to_string)
            .ok_or_else(|| AppError::Protocol("Graph /me returned no address".into()))
    }

    async fn mime(&self, id: &str) -> AppResult<Vec<u8>> {
        Ok(self
            .http
            .get(self.url(&format!("/messages/{id}/$value")))
            .bearer_auth(&self.token)
            .send()
            .await?
            .checked().await?
            .bytes()
            .await?
            .to_vec())
    }
}

#[async_trait]
impl IncomingProvider for GraphProvider {
    async fn test(&self) -> AppResult<()> {
        self.profile_email().await.map(|_| ())
    }

    async fn list_folders(&self) -> AppResult<Vec<RemoteFolder>> {
        let mut roles = std::collections::HashMap::new();
        for (wk, role) in WELL_KNOWN {
            if let Ok(f) = self.get_json::<MailFolder>(&format!("/mailFolders/{wk}")).await {
                roles.insert(f.id, *role);
            }
        }
        let mut out = Vec::new();
        let mut next = Some("/mailFolders?$top=100".to_string());
        while let Some(path) = next {
            let page: FolderList = self.get_json(&path).await?;
            for f in page.value {
                let role = roles.get(&f.id).copied().unwrap_or(FolderRole::Custom);
                out.push(RemoteFolder { name: f.display_name, path: f.id, role });
            }
            next = page.next_link;
        }
        Ok(out)
    }

    async fn fetch(&self, folder: &RemoteFolder, state: Option<&str>) -> AppResult<FetchResult> {
        let state: Option<GraphState> = state.and_then(|s| serde_json::from_str(s).ok());
        let initial = state.is_none();
        let mut next = Some(match state {
            Some(s) => s.delta_link,
            None => format!("/mailFolders/{}/messages/delta?$select=id,isRead,flag", folder.path),
        });

        let mut changed: Vec<(String, bool, bool)> = Vec::new();
        let mut delta_link = None;
        while let Some(url) = next.take() {
            let page: DeltaPage = self.get_json(&url).await?;
            for v in page.value {
                if v.get("@removed").is_some() {
                    continue;
                }
                let Some(id) = v.get("id").and_then(Value::as_str) else { continue };
                let is_read = v.get("isRead").and_then(Value::as_bool).unwrap_or(false);
                let flagged = v.pointer("/flag/flagStatus").and_then(Value::as_str) == Some("flagged");
                changed.push((id.to_string(), is_read, flagged));
            }
            next = page.next_link;
            delta_link = page.delta_link.or(delta_link);
        }
        // Delta returns oldest-first; keep the newest on initial sync.
        if initial && changed.len() > INITIAL_SYNC_LIMIT {
            changed.drain(..changed.len() - INITIAL_SYNC_LIMIT);
        }

        let mut messages = Vec::with_capacity(changed.len());
        for (id, is_read, is_flagged) in changed {
            match self.mime(&id).await {
                Ok(raw) => messages.push(FetchedMessage { remote_id: id, raw, is_read, is_flagged }),
                Err(AppError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(FetchResult {
            messages,
            new_state: delta_link.map(|delta_link| serde_json::to_string(&GraphState { delta_link })).transpose()?,
        })
    }

    async fn apply(&self, _folder_path: &str, remote_id: &str, action: &RemoteAction) -> AppResult<()> {
        let msg = format!("/messages/{remote_id}");
        let mv = |dest: &str| json!({ "destinationId": dest });
        match action {
            RemoteAction::MarkRead(r) => self.send_json(reqwest::Method::PATCH, &msg, json!({ "isRead": r })).await,
            RemoteAction::Flag(f) => {
                let status = if *f { "flagged" } else { "notFlagged" };
                self.send_json(reqwest::Method::PATCH, &msg, json!({ "flag": { "flagStatus": status } })).await
            }
            RemoteAction::Move { to_path } => self.send_json(reqwest::Method::POST, &format!("{msg}/move"), mv(to_path)).await,
            RemoteAction::Trash { .. } => {
                self.send_json(reqwest::Method::POST, &format!("{msg}/move"), mv("deleteditems")).await
            }
            RemoteAction::Archive { .. } => {
                self.send_json(reqwest::Method::POST, &format!("{msg}/move"), mv("archive")).await
            }
            RemoteAction::Delete => {
                self.http.delete(self.url(&msg)).bearer_auth(&self.token).send().await?.checked().await?;
                Ok(())
            }
        }
    }
}

#[async_trait]
impl OutgoingProvider for GraphProvider {
    async fn test(&self) -> AppResult<()> {
        self.profile_email().await.map(|_| ())
    }

    async fn send(&self, message: &lettre::Message) -> AppResult<()> {
        // Graph accepts a base64-encoded MIME body with text/plain content type.
        self.http
            .post(self.url("/sendMail"))
            .bearer_auth(&self.token)
            .header("Content-Type", "text/plain")
            .body(STANDARD.encode(message.formatted()))
            .send()
            .await?
            .checked().await?;
        Ok(())
    }
}
