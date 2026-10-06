//! Background mail synchronization engine.
//!
//! A single Tokio task owns the sync loop: it wakes up on a fixed interval or
//! when the UI requests a sync (`SyncHandle::request`). Accounts are synced
//! concurrently; each folder is synced incrementally using the provider's
//! persisted cursor. New unread inbox mail is announced via `mail:new`.

use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use serde_json::json;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::mime;
use crate::models::{Account, FolderRole, SyncReport};
use crate::providers;
use crate::service::{events, AppContext};

pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq)]
pub enum SyncCommand {
    All,
    Account(String),
}

#[derive(Clone)]
pub struct SyncHandle {
    tx: mpsc::Sender<SyncCommand>,
}

impl SyncHandle {
    /// Queues a sync. Never blocks; drops the request if the queue is full,
    /// which is fine because a sync is already pending.
    pub fn request(&self, cmd: SyncCommand) {
        let _ = self.tx.try_send(cmd);
    }
}

pub struct SyncEngine {
    ctx: Arc<AppContext>,
    rx: mpsc::Receiver<SyncCommand>,
    interval: Duration,
}

impl SyncEngine {
    pub fn new(ctx: Arc<AppContext>, interval: Duration) -> (Self, SyncHandle) {
        let (tx, rx) = mpsc::channel(16);
        (Self { ctx, rx, interval }, SyncHandle { tx })
    }

    /// Runs until every `SyncHandle` is dropped.
    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(self.interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => self.sync_all().await,
                cmd = self.rx.recv() => match cmd {
                    None => break,
                    Some(SyncCommand::All) => self.sync_all().await,
                    Some(SyncCommand::Account(id)) => {
                        match self.ctx.db.get_account(&id) {
                            Ok(account) => { run_one(&self.ctx, &account).await; }
                            Err(e) => log::warn!("sync request for unknown account {id}: {e}"),
                        }
                    }
                },
            }
        }
    }

    async fn sync_all(&self) {
        let accounts = match self.ctx.db.list_accounts() {
            Ok(a) => a,
            Err(e) => {
                log::error!("cannot list accounts: {e}");
                return;
            }
        };
        join_all(accounts.iter().map(|a| run_one(&self.ctx, a))).await;
    }
}

/// Syncs one account and reports progress through events.
async fn run_one(ctx: &AppContext, account: &Account) -> Option<SyncReport> {
    ctx.events.emit(events::SYNC_STATUS, json!({ "accountId": account.id, "state": "syncing" }));
    match sync_account(ctx, account).await {
        Ok(report) => {
            ctx.events.emit(
                events::SYNC_STATUS,
                json!({ "accountId": account.id, "state": "idle", "newMessages": report.new_messages }),
            );
            Some(report)
        }
        Err(e) => {
            log::warn!("sync of {} failed: {e}", account.email);
            ctx.events.emit(
                events::SYNC_STATUS,
                json!({ "accountId": account.id, "state": "error", "error": e.to_string() }),
            );
            None
        }
    }
}

/// Parses and stores fetched messages; returns the ids (and read state) of
/// the ones that were new.
fn store_messages(
    ctx: &AppContext,
    account: &Account,
    folder: &crate::models::Folder,
    messages: Vec<providers::FetchedMessage>,
) -> AppResult<Vec<(String, bool)>> {
    let mut added = Vec::new();
    for m in messages {
        let parsed = match mime::parse_message(&m.raw) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("skipping unparseable message {}: {e}", m.remote_id);
                continue;
            }
        };
        let new = parsed.into_new_email(&account.id, &folder.id, &m.remote_id, m.is_read, m.is_flagged);
        if let Some(id) = ctx.db.insert_email(&new)? {
            added.push((id, m.is_read));
        }
    }
    Ok(added)
}

/// Older messages of a folder, fetched when its list is scrolled to the end.
/// They are not announced as new mail.
pub async fn load_older(ctx: &AppContext, folder_id: &str) -> AppResult<crate::models::OlderPage> {
    let folder = ctx.db.get_folder(folder_id)?;
    let account = ctx.db.get_account(&folder.account_id)?;
    let creds = ctx.credentials(&account).await?;
    let provider = providers::incoming_for(&account, &creds)?;
    let remote = providers::RemoteFolder { name: folder.name.clone(), path: folder.path.clone(), role: folder.role };
    let state = ctx.db.get_sync_state(&folder.id)?;
    let known = ctx.db.remote_ids(&folder.id)?;
    let result = provider.fetch_older(&remote, state.as_deref(), &known, providers::gmail::PAGE).await?;
    let added = store_messages(ctx, &account, &folder, result.messages)?.len() as u32;
    if let Some(s) = result.new_state {
        ctx.db.set_sync_state(&folder.id, &s)?;
    }
    if added > 0 {
        ctx.events.emit(events::MAIL_CHANGED, json!({ "accountId": account.id }));
    }
    Ok(crate::models::OlderPage { added, has_more: !result.exhausted })
}

/// One full incremental pass over every folder of an account.
pub async fn sync_account(ctx: &AppContext, account: &Account) -> AppResult<SyncReport> {
    let creds = ctx.credentials(account).await?;
    let provider = providers::incoming_for(account, &creds)?;

    let mut remote = provider.list_folders().await?;
    remote.sort_by_key(|f| f.role.sort_key());

    let mut new_messages = 0u32;
    let mut new_inbox_ids = Vec::new();
    for rf in &remote {
        let folder = ctx.db.upsert_remote_folder(&account.id, &rf.name, &rf.path, rf.role)?;
        let state = ctx.db.get_sync_state(&folder.id)?;
        let known = ctx.db.remote_ids(&folder.id)?;
        let result = match provider.fetch_new(rf, state.as_deref(), &known).await {
            Ok(r) => r,
            // Credentials problems affect every folder — abort the account.
            Err(e @ AppError::Auth(_)) => return Err(e),
            Err(e) => {
                log::warn!("folder {} of {} failed: {e}", rf.path, account.email);
                continue;
            }
        };
        for (id, is_read) in store_messages(ctx, account, &folder, result.messages)? {
            new_messages += 1;
            if folder.role == FolderRole::Inbox && !is_read {
                new_inbox_ids.push(id);
            }
        }
        if let Some(s) = result.new_state {
            ctx.db.set_sync_state(&folder.id, &s)?;
        }
    }

    if new_messages > 0 {
        ctx.events.emit(events::MAIL_CHANGED, json!({ "accountId": account.id }));
    }
    if !new_inbox_ids.is_empty() {
        let latest: Vec<_> = new_inbox_ids
            .iter()
            .rev()
            .take(5)
            .filter_map(|id| ctx.db.get_email(id).ok().map(|e| e.summary))
            .collect();
        ctx.events.emit(
            events::MAIL_NEW,
            json!({ "accountId": account.id, "count": new_inbox_ids.len(), "latest": latest }),
        );
    }
    Ok(SyncReport { account_id: account.id.clone(), new_messages, folders: remote.len() as u32 })
}
