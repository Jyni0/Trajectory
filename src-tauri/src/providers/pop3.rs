//! POP3 (RFC 1939) client + provider.
//!
//! The protocol is tiny, so the client is implemented directly over tokio
//! streams: USER/PASS, STLS, UIDL, TOP, RETR, DELE, QUIT. Messages are left on
//! the server; the UIDL set acts as the delta-sync cursor.

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::{FetchResult, FetchedMessage, IncomingProvider, RemoteAction, RemoteFolder, INITIAL_SYNC_LIMIT};
use crate::error::{AppError, AppResult};
use crate::models::{FolderRole, Security, ServerConfig};
use crate::net::{self, BoxStream};

const MAX_LINE: usize = 64 * 1024;

pub struct Pop3Client<S> {
    stream: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Pop3Client<S> {
    /// Wraps a fresh connection and consumes the server greeting.
    pub async fn connect(stream: S) -> AppResult<Self> {
        let mut c = Self { stream: BufReader::new(stream) };
        c.expect_ok().await?;
        Ok(c)
    }

    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    async fn read_line(&mut self) -> AppResult<Vec<u8>> {
        let mut line = Vec::new();
        let n = self.stream.read_until(b'\n', &mut line).await?;
        if n == 0 {
            return Err(AppError::Network("POP3 server closed the connection".into()));
        }
        if line.len() > MAX_LINE {
            return Err(AppError::Protocol("POP3 line too long".into()));
        }
        Ok(line)
    }

    async fn expect_ok(&mut self) -> AppResult<String> {
        let line = self.read_line().await?;
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        if let Some(rest) = text.strip_prefix("+OK") {
            Ok(rest.trim().to_string())
        } else {
            let msg = text.strip_prefix("-ERR").unwrap_or(&text).trim().to_string();
            Err(AppError::Protocol(format!("POP3: {msg}")))
        }
    }

    pub async fn command(&mut self, cmd: &str) -> AppResult<String> {
        let w = self.stream.get_mut();
        w.write_all(cmd.as_bytes()).await?;
        w.write_all(b"\r\n").await?;
        w.flush().await?;
        self.expect_ok().await
    }

    /// Reads a dot-terminated multi-line response, undoing dot-stuffing.
    async fn read_multiline(&mut self) -> AppResult<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let line = self.read_line().await?;
            if line == b".\r\n" || line == b".\n" {
                return Ok(out);
            }
            let content = if line.starts_with(b"..") { &line[1..] } else { &line[..] };
            out.extend_from_slice(content);
        }
    }

    pub async fn login(&mut self, user: &str, pass: &str) -> AppResult<()> {
        self.command(&format!("USER {user}")).await?;
        self.command(&format!("PASS {pass}"))
            .await
            .map_err(|e| AppError::Auth(format!("POP3 login rejected: {e}")))?;
        Ok(())
    }

    pub async fn stat(&mut self) -> AppResult<(u32, u64)> {
        let resp = self.command("STAT").await?;
        let mut parts = resp.split_whitespace();
        let count = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let size = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((count, size))
    }

    /// `(message number, unique id)` for every message in the maildrop.
    pub async fn uidl(&mut self) -> AppResult<Vec<(u32, String)>> {
        self.command("UIDL").await?;
        let body = self.read_multiline().await?;
        Ok(String::from_utf8_lossy(&body)
            .lines()
            .filter_map(|l| {
                let mut p = l.split_whitespace();
                Some((p.next()?.parse().ok()?, p.next()?.to_string()))
            })
            .collect())
    }

    pub async fn retr(&mut self, n: u32) -> AppResult<Vec<u8>> {
        self.command(&format!("RETR {n}")).await?;
        self.read_multiline().await
    }

    pub async fn top(&mut self, n: u32, lines: u32) -> AppResult<Vec<u8>> {
        self.command(&format!("TOP {n} {lines}")).await?;
        self.read_multiline().await
    }

    pub async fn dele(&mut self, n: u32) -> AppResult<()> {
        self.command(&format!("DELE {n}")).await.map(|_| ())
    }

    pub async fn quit(mut self) -> AppResult<()> {
        self.command("QUIT").await.map(|_| ())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pop3State {
    pub seen: Vec<String>,
}

pub struct Pop3Provider {
    cfg: ServerConfig,
    username: String,
    password: String,
    download_days: Option<u32>,
}

impl Pop3Provider {
    pub fn new(cfg: ServerConfig, username: String, password: String, download_days: Option<u32>) -> Self {
        Self { cfg, username, password, download_days }
    }

    async fn connect(&self) -> AppResult<Pop3Client<BoxStream>> {
        let host = self.cfg.host.as_str();
        let tcp = net::tcp_connect(host, self.cfg.port).await?;
        let mut client = match self.cfg.security {
            Security::Tls => Pop3Client::connect(Box::new(net::tls_upgrade(host, tcp).await?) as BoxStream).await?,
            Security::None => Pop3Client::connect(Box::new(tcp) as BoxStream).await?,
            Security::StartTls => {
                let mut plain = Pop3Client::connect(tcp).await?;
                plain.command("STLS").await?;
                let tls = net::tls_upgrade(host, plain.into_inner()).await?;
                // No greeting after STLS.
                Pop3Client { stream: BufReader::new(Box::new(tls) as BoxStream) }
            }
        };
        client.login(&self.username, &self.password).await?;
        Ok(client)
    }
}

/// Extracts the Date header timestamp from a header block (TOP n 0).
fn header_timestamp(headers: &[u8]) -> Option<i64> {
    mail_parser::MessageParser::default()
        .parse_headers(headers)
        .and_then(|m| m.date().map(|d| d.to_timestamp()))
}

#[async_trait]
impl IncomingProvider for Pop3Provider {
    async fn test(&self) -> AppResult<()> {
        let mut c = self.connect().await?;
        c.stat().await?;
        c.quit().await
    }

    async fn list_folders(&self) -> AppResult<Vec<RemoteFolder>> {
        Ok(vec![RemoteFolder { name: "Inbox".into(), path: "INBOX".into(), role: FolderRole::Inbox }])
    }

    async fn fetch(&self, folder: &RemoteFolder, state: Option<&str>) -> AppResult<FetchResult> {
        if folder.role != FolderRole::Inbox {
            return Ok(FetchResult::default());
        }
        let state: Pop3State = state.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        let mut seen: HashSet<String> = state.seen.into_iter().collect();
        let mut c = self.connect().await?;
        let list = c.uidl().await?;
        let cutoff = self.download_days.map(|d| chrono::Utc::now().timestamp() - i64::from(d) * 86_400);

        let fresh: Vec<&(u32, String)> = list.iter().filter(|(_, uid)| !seen.contains(uid)).collect();
        // Newest messages have the highest numbers; cap the initial download.
        let start = fresh.len().saturating_sub(INITIAL_SYNC_LIMIT);
        let mut messages = Vec::new();
        for (n, uid) in &fresh[start..] {
            if let Some(cutoff) = cutoff {
                let headers = c.top(*n, 0).await?;
                if header_timestamp(&headers).is_some_and(|ts| ts < cutoff) {
                    seen.insert(uid.clone());
                    continue;
                }
            }
            let raw = c.retr(*n).await?;
            messages.push(FetchedMessage { remote_id: uid.clone(), raw, is_read: false, is_flagged: false });
            seen.insert(uid.clone());
        }
        c.quit().await?;

        // Forget UIDLs that are no longer on the server to keep the state small.
        let present: HashSet<&String> = list.iter().map(|(_, u)| u).collect();
        let seen: Vec<String> = seen.into_iter().filter(|u| present.contains(u)).collect();
        Ok(FetchResult { messages, new_state: Some(serde_json::to_string(&Pop3State { seen })?) })
    }

    async fn apply(&self, _folder_path: &str, remote_id: &str, action: &RemoteAction) -> AppResult<()> {
        // POP3 has no flags or folders; only permanent deletion reaches the server.
        if *action != RemoteAction::Delete {
            return Ok(());
        }
        let mut c = self.connect().await?;
        if let Some((n, _)) = c.uidl().await?.into_iter().find(|(_, uid)| uid == remote_id) {
            c.dele(n).await?;
        }
        c.quit().await
    }
}
