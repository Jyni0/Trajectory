//! IMAP4rev1 provider built on `async-imap`.
//!
//! Delta sync uses the classic UIDVALIDITY + last-seen UID cursor: each sync
//! fetches `UID FETCH <last+1>:*` and resets when UIDVALIDITY changes.

use async_imap::types::{Flag, NameAttribute};
use async_trait::async_trait;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};

use super::{FetchResult, FetchedMessage, IncomingProvider, RemoteAction, RemoteFolder, INITIAL_SYNC_LIMIT};
use crate::error::{AppError, AppResult};
use crate::models::{FolderRole, Security, ServerConfig};
use crate::net::{self, BoxStream};

type Session = async_imap::Session<BoxStream>;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImapState {
    pub uid_validity: u32,
    pub last_uid: u32,
}

pub struct ImapProvider {
    cfg: ServerConfig,
    username: String,
    password: String,
}

impl ImapProvider {
    pub fn new(cfg: ServerConfig, username: String, password: String) -> Self {
        Self { cfg, username, password }
    }

    async fn connect(&self) -> AppResult<Session> {
        let host = self.cfg.host.as_str();
        let tcp = net::tcp_connect(host, self.cfg.port).await?;
        let stream: BoxStream = match self.cfg.security {
            Security::Tls => Box::new(net::tls_upgrade(host, tcp).await?),
            Security::None => Box::new(tcp),
            Security::StartTls => {
                let mut plain = async_imap::Client::new(tcp);
                read_greeting(&mut plain).await?;
                plain.run_command_and_check_ok("STARTTLS", None).await?;
                Box::new(net::tls_upgrade(host, plain.into_inner()).await?)
            }
        };
        let mut client = async_imap::Client::new(stream);
        if self.cfg.security != Security::StartTls {
            read_greeting(&mut client).await?;
        }
        client
            .login(&self.username, &self.password)
            .await
            .map_err(|(e, _)| AppError::Auth(format!("IMAP login rejected: {e}")))
    }
}

async fn read_greeting<T>(client: &mut async_imap::Client<T>) -> AppResult<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    match client.read_response().await {
        Some(Ok(_)) => Ok(()),
        Some(Err(e)) => Err(AppError::Network(e.to_string())),
        None => Err(AppError::Protocol("server closed the connection before greeting".into())),
    }
}

fn role_from_attributes(attrs: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Junk => Some(FolderRole::Spam),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Archive | NameAttribute::All => Some(FolderRole::Archive),
        _ => None,
    })
}

#[async_trait]
impl IncomingProvider for ImapProvider {
    async fn test(&self) -> AppResult<()> {
        let mut session = self.connect().await?;
        session.select("INBOX").await?;
        let _ = session.logout().await;
        Ok(())
    }

    async fn list_folders(&self) -> AppResult<Vec<RemoteFolder>> {
        let mut session = self.connect().await?;
        let names: Vec<_> = session.list(Some(""), Some("*")).await?.try_collect().await?;
        let mut folders = Vec::new();
        for n in &names {
            if n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
                continue;
            }
            let path = n.name().to_string();
            let display = decode_modified_utf7(path.rsplit(n.delimiter().unwrap_or("/")).next().unwrap_or(&path));
            let role = if path.eq_ignore_ascii_case("INBOX") {
                FolderRole::Inbox
            } else {
                role_from_attributes(n.attributes()).unwrap_or_else(|| super::role_from_name(&display))
            };
            folders.push(RemoteFolder { name: display, path, role });
        }
        let _ = session.logout().await;
        Ok(folders)
    }

    async fn fetch(&self, folder: &RemoteFolder, state: Option<&str>) -> AppResult<FetchResult> {
        let mut state: ImapState = state.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        let mut session = self.connect().await?;
        let mailbox = session.select(&folder.path).await?;
        let uid_validity = mailbox.uid_validity.unwrap_or(0);
        if uid_validity != state.uid_validity {
            state = ImapState { uid_validity, last_uid: 0 };
        }

        const QUERY: &str = "(UID FLAGS BODY.PEEK[])";
        let fetches: Vec<_> = if state.last_uid == 0 {
            if mailbox.exists == 0 {
                Vec::new()
            } else {
                let from = mailbox.exists.saturating_sub(INITIAL_SYNC_LIMIT as u32 - 1).max(1);
                session.fetch(format!("{from}:*"), QUERY).await?.try_collect().await?
            }
        } else {
            session
                .uid_fetch(format!("{}:*", state.last_uid + 1), QUERY)
                .await?
                .try_collect()
                .await?
        };

        let mut messages = Vec::with_capacity(fetches.len());
        for f in &fetches {
            let Some(uid) = f.uid else { continue };
            // `n:*` always returns the last message, even if its UID < n.
            if uid <= state.last_uid {
                continue;
            }
            let Some(body) = f.body() else { continue };
            let flags: Vec<Flag<'_>> = f.flags().collect();
            messages.push(FetchedMessage {
                remote_id: uid.to_string(),
                raw: body.to_vec(),
                is_read: flags.contains(&Flag::Seen),
                is_flagged: flags.contains(&Flag::Flagged),
            });
        }
        if let Some(max) = messages.iter().filter_map(|m| m.remote_id.parse::<u32>().ok()).max() {
            state.last_uid = state.last_uid.max(max);
        }
        let _ = session.logout().await;
        Ok(FetchResult { messages, new_state: Some(serde_json::to_string(&state)?) })
    }

    async fn apply(&self, folder_path: &str, remote_id: &str, action: &RemoteAction) -> AppResult<()> {
        let uid: u32 = remote_id
            .parse()
            .map_err(|_| AppError::Invalid(format!("not an IMAP UID: {remote_id}")))?;
        let mut session = self.connect().await?;
        session.select(folder_path).await?;

        async fn store(s: &mut Session, uid: u32, q: &str) -> AppResult<()> {
            let _: Vec<_> = s.uid_store(uid.to_string(), q).await?.try_collect().await?;
            Ok(())
        }
        async fn expunge(s: &mut Session, uid: u32) -> AppResult<()> {
            store(s, uid, "+FLAGS.SILENT (\\Deleted)").await?;
            let _: Vec<_> = s.expunge().await?.try_collect().await?;
            Ok(())
        }
        async fn move_to(s: &mut Session, uid: u32, to: &str) -> AppResult<()> {
            if s.uid_mv(uid.to_string(), to).await.is_err() {
                // Server without the MOVE extension: COPY + delete.
                s.uid_copy(uid.to_string(), to).await?;
                expunge(s, uid).await?;
            }
            Ok(())
        }

        match action {
            RemoteAction::MarkRead(true) => store(&mut session, uid, "+FLAGS.SILENT (\\Seen)").await?,
            RemoteAction::MarkRead(false) => store(&mut session, uid, "-FLAGS.SILENT (\\Seen)").await?,
            RemoteAction::Flag(true) => store(&mut session, uid, "+FLAGS.SILENT (\\Flagged)").await?,
            RemoteAction::Flag(false) => store(&mut session, uid, "-FLAGS.SILENT (\\Flagged)").await?,
            RemoteAction::Move { to_path } => move_to(&mut session, uid, to_path).await?,
            RemoteAction::Trash { trash_path: Some(p) } | RemoteAction::Archive { archive_path: Some(p) } => {
                move_to(&mut session, uid, p).await?
            }
            RemoteAction::Trash { trash_path: None } | RemoteAction::Delete => expunge(&mut session, uid).await?,
            RemoteAction::Archive { archive_path: None } => {}
        }
        let _ = session.logout().await;
        Ok(())
    }

    async fn append(&self, folder_path: &str, raw: &[u8]) -> AppResult<()> {
        let mut session = self.connect().await?;
        session.append(folder_path, Some("(\\Seen)"), None, raw).await?;
        let _ = session.logout().await;
        Ok(())
    }
}

/// Decodes IMAP "modified UTF-7" mailbox names (RFC 3501 §5.1.3),
/// e.g. `&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-` → `Отправленные`.
pub fn decode_modified_utf7(input: &str) -> String {
    use base64::Engine;
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let b64 = chunk.replace(',', "/");
            match base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.as_bytes()) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => out.push_str(&rest[start..start + end + 2]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}
