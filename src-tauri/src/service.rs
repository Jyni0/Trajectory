//! Application service: everything the IPC commands do, independent of Tauri
//! so it can be exercised directly from tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::Message;
use serde_json::json;
use uuid::Uuid;

use crate::db::{Database, NewAttachment, NewEmail};
use crate::error::{AppError, AppResult};
use crate::mime;
use crate::models::*;
use crate::oauth::{self, OAuthProvider, TokenCache};
use crate::providers::{self, Credentials, RemoteAction};
use crate::secrets::{SecretStore, KEY_PASSWORD, KEY_REFRESH_TOKEN};

/// Abstraction over `AppHandle::emit` so the core can run headless in tests.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

pub mod events {
    pub const MAIL_NEW: &str = "mail:new";
    pub const MAIL_CHANGED: &str = "mail:changed";
    pub const SYNC_STATUS: &str = "sync:status";
    pub const SYNC_ERROR: &str = "sync:error";
}

const ACCOUNT_COLORS: &[&str] = &["#2563eb", "#7c3aed", "#db2777", "#ea580c", "#16a34a", "#0891b2", "#ca8a04"];

pub struct AppContext {
    pub db: Database,
    pub secrets: Arc<dyn SecretStore>,
    pub tokens: TokenCache,
    pub events: Arc<dyn EventSink>,
    /// DNS + HTTPS used for BIMI logos; created on first use.
    pub bimi: tokio::sync::OnceCell<(Arc<dyn crate::bimi::Dns>, Arc<dyn crate::bimi::Fetch>)>,
}

/// How long a looked-up logo is trusted before it is checked again.
const LOGO_TTL: i64 = 7 * 24 * 3600;
/// Domains without a logo are asked again sooner.
const NO_LOGO_TTL: i64 = 24 * 3600;

/// An attachment's name made safe to create in the Downloads folder: no path
/// separators or reserved characters, no `.`/`..`, no trailing dots/spaces
/// and no Windows device names (CON, NUL, COM1…).
pub fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    let stem = cleaned.split('.').next().unwrap_or_default().trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    let cleaned: String = cleaned.chars().take(180).collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "attachment".to_string()
    } else if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

impl AppContext {
    pub fn new(db: Database, secrets: Arc<dyn SecretStore>, events: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self { db, secrets, tokens: TokenCache::default(), events, bimi: tokio::sync::OnceCell::new() })
    }

    pub async fn credentials(&self, account: &Account) -> AppResult<Credentials> {
        if account.kind.is_oauth() {
            let access_token = self.tokens.access_token(&account.id, account.kind, self.secrets.as_ref()).await?;
            Ok(Credentials::OAuth { access_token })
        } else {
            Ok(Credentials::Password(self.secrets.require(&account.id, KEY_PASSWORD)?))
        }
    }

    // ───────────────────────────── accounts ─────────────────────────────

    pub fn list_accounts(&self) -> AppResult<Vec<Account>> {
        self.db.list_accounts()
    }

    fn next_color(&self) -> AppResult<String> {
        let n = self.db.list_accounts()?.len();
        Ok(ACCOUNT_COLORS[n % ACCOUNT_COLORS.len()].to_string())
    }

    fn account_from_new(&self, new: &NewAccount) -> AppResult<Account> {
        Ok(Account {
            id: Uuid::new_v4().to_string(),
            kind: new.kind,
            display_name: if new.display_name.trim().is_empty() {
                new.email.split('@').next().unwrap_or_default().to_string()
            } else {
                new.display_name.trim().to_string()
            },
            email: new.email.trim().to_string(),
            incoming: new.incoming.clone(),
            outgoing: new.outgoing.clone(),
            username: new.username.clone().filter(|u| !u.trim().is_empty()),
            pop3_download_days: if new.kind == AccountKind::Pop3 { new.pop3_download_days } else { None },
            color: self.next_color()?,
            created_at: chrono::Utc::now().timestamp(),
        })
    }

    fn validate_new(new: &NewAccount) -> AppResult<()> {
        crate::autodetect::domain_of(&new.email)?;
        if new.kind.is_oauth() {
            return Err(AppError::Invalid("use the OAuth sign-in for Google and Microsoft accounts".into()));
        }
        let check = |c: &Option<ServerConfig>, what: &str| match c {
            Some(c) if !c.host.trim().is_empty() && c.port != 0 => Ok(()),
            _ => Err(AppError::Invalid(format!("{what} server host and port are required"))),
        };
        check(&new.incoming, if new.kind == AccountKind::Pop3 { "POP3" } else { "IMAP" })?;
        check(&new.outgoing, "SMTP")?;
        // Without TLS the password crosses the network in clear text: only
        // allowed towards this machine or a private network.
        for c in [&new.incoming, &new.outgoing].into_iter().flatten() {
            if c.security == Security::None && !crate::net::is_private_host(&c.host) {
                return Err(AppError::Invalid(format!(
                    "{} is a public server: an unencrypted connection would expose the password. Use SSL/TLS or STARTTLS.",
                    c.host.trim()
                )));
            }
        }
        if new.password.as_deref().unwrap_or_default().is_empty() {
            return Err(AppError::Invalid("password is required".into()));
        }
        Ok(())
    }

    /// Checks incoming and outgoing servers with the supplied credentials.
    pub async fn test_connection(&self, new: &NewAccount) -> AppResult<ConnectionReport> {
        Self::validate_new(new)?;
        let account = Account { color: String::new(), ..self.account_from_new(new)? };
        let creds = Credentials::Password(new.password.clone().unwrap_or_default());
        let incoming = providers::incoming_for(&account, &creds)?;
        let outgoing = providers::outgoing_for(&account, &creds)?;
        let (inc, out) = tokio::join!(incoming.test(), outgoing.test());
        Ok(ConnectionReport {
            incoming_ok: inc.is_ok(),
            outgoing_ok: out.is_ok(),
            incoming_error: inc.err().map(|e| e.to_string()),
            outgoing_error: out.err().map(|e| e.to_string()),
        })
    }

    /// Adds a password-based (IMAP/POP3 + SMTP) account. With `verify`, the
    /// account is only saved when both servers accept the credentials.
    pub async fn add_account(&self, new: NewAccount, verify: bool) -> AppResult<Account> {
        Self::validate_new(&new)?;
        if self.db.find_account_by_email(new.email.trim())?.is_some() {
            return Err(AppError::Invalid(format!("{} is already added", new.email.trim())));
        }
        if verify {
            let report = self.test_connection(&new).await?;
            if !report.ok() {
                let msg = report.incoming_error.or(report.outgoing_error).unwrap_or_default();
                return Err(AppError::Auth(msg));
            }
        }
        let account = self.account_from_new(&new)?;
        self.db.insert_account(&account)?;
        if let Err(e) = self.secrets.set(&account.id, KEY_PASSWORD, new.password.as_deref().unwrap_or_default()) {
            let _ = self.db.delete_account(&account.id);
            return Err(e);
        }
        self.ensure_system_folders(&account.id)?;
        Ok(account)
    }

    /// The OAuth apps (Google, Microsoft) as Settings shows them.
    pub fn oauth_apps(&self) -> AppResult<Vec<oauth::OAuthAppInfo>> {
        [OAuthProvider::Google, OAuthProvider::Microsoft].iter().map(|p| p.info(self.secrets.as_ref())).collect()
    }

    /// Saves an OAuth app in the keychain; an empty secret keeps the saved one.
    pub fn set_oauth_app(&self, provider: &str, client_id: &str, client_secret: Option<&str>) -> AppResult<oauth::OAuthAppInfo> {
        let p = OAuthProvider::parse(provider)?;
        p.save_app(self.secrets.as_ref(), client_id, client_secret)?;
        p.info(self.secrets.as_ref())
    }

    pub fn clear_oauth_app(&self, provider: &str) -> AppResult<oauth::OAuthAppInfo> {
        let p = OAuthProvider::parse(provider)?;
        p.clear_app(self.secrets.as_ref())?;
        p.info(self.secrets.as_ref())
    }

    /// Interactive OAuth sign-in. Re-authorizing an existing mailbox just
    /// replaces its refresh token.
    pub async fn add_oauth_account<F>(&self, kind: AccountKind, open_browser: F) -> AppResult<Account>
    where
        F: FnOnce(&str) -> AppResult<()>,
    {
        let provider = OAuthProvider::for_kind(kind)?;
        let app = provider.app(self.secrets.as_ref())?;
        let (grant, email) = oauth::authorize(provider, &app, open_browser).await?;
        let refresh_token = grant.refresh_token.clone().unwrap_or_default();

        if let Some(existing) = self.db.find_account_by_email(&email)? {
            self.secrets.set(&existing.id, KEY_REFRESH_TOKEN, &refresh_token)?;
            self.tokens.put(&existing.id, &grant);
            return Ok(existing);
        }
        let account = Account {
            id: Uuid::new_v4().to_string(),
            kind,
            display_name: email.split('@').next().unwrap_or_default().to_string(),
            email,
            incoming: None,
            outgoing: None,
            username: None,
            pop3_download_days: None,
            color: self.next_color()?,
            created_at: chrono::Utc::now().timestamp(),
        };
        self.db.insert_account(&account)?;
        self.secrets.set(&account.id, KEY_REFRESH_TOKEN, &refresh_token)?;
        self.tokens.put(&account.id, &grant);
        self.ensure_system_folders(&account.id)?;
        Ok(account)
    }

    pub fn remove_account(&self, id: &str) -> AppResult<()> {
        self.db.delete_account(id)?;
        self.tokens.forget(id);
        // Keychain cleanup failures must not resurrect the account.
        if let Err(e) = self.secrets.delete_all(id) {
            log::warn!("could not remove secrets for {id}: {e}");
        }
        Ok(())
    }

    fn ensure_system_folders(&self, account_id: &str) -> AppResult<()> {
        for role in [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts, FolderRole::Trash] {
            self.db.ensure_folder(account_id, role)?;
        }
        Ok(())
    }

    // ───────────────────────────── reading ──────────────────────────────

    pub fn list_folders(&self, account_id: &str) -> AppResult<Vec<Folder>> {
        self.db.list_folders(account_id)
    }

    pub fn list_emails(&self, query: &EmailQuery) -> AppResult<EmailPage> {
        self.db.list_emails(query)
    }

    /// Fetches the next batch of older messages of a folder from the server.
    pub async fn load_older(&self, folder_id: &str) -> AppResult<OlderPage> {
        crate::mail_engine::load_older(self, folder_id).await
    }

    pub fn thread(&self, id: &str) -> AppResult<Vec<EmailSummary>> {
        self.db.thread(id)
    }

    pub fn get_email(&self, id: &str) -> AppResult<Email> {
        let mut email = self.db.get_email(id)?;
        if let Some(html) = &email.body_html {
            if html.contains("cid:") {
                email.body_html = Some(mime::inline_cid_images(html, &self.db.inline_parts(id)?));
            }
        }
        email.attachments.retain(|a| a.content_id.is_none());
        Ok(email)
    }

    pub fn save_attachment(&self, id: &str, dir: &Path) -> AppResult<PathBuf> {
        let (meta, data) = self.db.get_attachment(id)?;
        std::fs::create_dir_all(dir)?;
        let safe = safe_filename(&meta.filename);
        let (stem, ext) = match safe.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (safe.clone(), String::new()),
        };
        // `create_new` never follows or overwrites an existing file (or a
        // link planted in its place); on a clash the next free name is tried.
        for n in 0..1000 {
            let name = if n == 0 { safe.clone() } else { format!("{stem} ({n}){ext}") };
            let path = dir.join(name);
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    f.write_all(&data)?;
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::Invalid(format!("no free file name for {safe}")))
    }

    // ─────────────────────────── sender logos ───────────────────────────

    /// The BIMI logo of a sender domain, cached in SQLite. Only call it with a
    /// domain that passed DMARC for the message (`EmailSummary::logo_domain`).
    pub async fn sender_logo(&self, domain: &str) -> AppResult<SenderLogo> {
        let domain = domain.trim().to_ascii_lowercase();
        let now = chrono::Utc::now().timestamp();
        if let Some((logo, at)) = self.db.cached_logo(&domain)? {
            let ttl = if logo.svg.is_some() { LOGO_TTL } else { NO_LOGO_TTL };
            if now - at < ttl {
                return Ok(logo);
            }
        }
        let (dns, http) = self
            .bimi
            .get_or_try_init(|| async {
                let dns: Arc<dyn crate::bimi::Dns> = Arc::new(crate::bimi::SystemDns::new()?);
                let http: Arc<dyn crate::bimi::Fetch> = Arc::new(crate::bimi::HttpsFetch::new()?);
                AppResult::Ok((dns, http))
            })
            .await?;
        let when = rustls_pki_types::UnixTime::since_unix_epoch(std::time::Duration::from_secs(now.max(0) as u64));
        // A network failure is not cached, so the next view retries; a broken
        // or missing logo is cached like "no logo".
        let mut logo = match crate::bimi::lookup(dns.as_ref(), http.as_ref(), &domain, when).await {
            Ok(l) => l,
            Err(e @ AppError::Network(_)) => return Err(e),
            Err(e) => {
                log::info!("no BIMI logo for {domain}: {e}");
                SenderLogo::default()
            }
        };
        // No BIMI logo: the site's own icon, shown without the check mark.
        if logo.svg.is_none() {
            logo.svg = match crate::site_icon::lookup(http.as_ref(), &domain).await {
                Ok(icon) => icon,
                Err(e @ AppError::Network(_)) => return Err(e),
                Err(e) => {
                    log::info!("no site icon for {domain}: {e}");
                    None
                }
            };
        }
        self.db.store_logo(&domain, &logo, now)?;
        Ok(logo)
    }

    // ─────────────────────────── tags & notes ───────────────────────────

    pub fn list_tags(&self, account_id: Option<&str>) -> AppResult<Vec<Tag>> {
        self.db.list_tags(account_id)
    }

    pub fn create_tag(&self, name: &str, color: &str) -> AppResult<Tag> {
        self.db.create_tag(name, color)
    }

    pub fn update_tag(&self, id: &str, name: &str, color: &str) -> AppResult<Tag> {
        self.db.update_tag(id, name, color)
    }

    pub fn delete_tag(&self, id: &str) -> AppResult<()> {
        self.db.delete_tag(id)
    }

    pub fn reorder_tags(&self, ids: &[String]) -> AppResult<()> {
        self.db.reorder_tags(ids)
    }

    pub fn set_email_tag(&self, email_id: &str, tag_id: &str, on: bool) -> AppResult<()> {
        self.db.set_email_tag(email_id, tag_id, on)
    }

    pub fn set_note(&self, email_id: &str, note: Option<&str>) -> AppResult<()> {
        self.db.set_note(email_id, note)
    }

    // ───────────────────────────── actions ──────────────────────────────

    /// Replays a user action on the server in the background. Local state is
    /// already updated; failures surface as `sync:error` events.
    fn push_remote(self: &Arc<Self>, email_id: &str, action: RemoteAction) -> AppResult<()> {
        let loc = self.db.email_location(email_id)?;
        let account = self.db.get_account(&loc.account_id)?;
        // POP3 ids are folder-independent, so mail filed into device-only
        // folders can still be deleted from the maildrop.
        let reachable = account.kind == AccountKind::Pop3 || !loc.folder.is_local();
        if !reachable || loc.remote_id.starts_with("local-") {
            return Ok(());
        }
        let ctx = Arc::clone(self);
        tokio::spawn(async move {
            let result = async {
                let creds = ctx.credentials(&account).await?;
                let provider = providers::incoming_for(&account, &creds)?;
                provider.apply(&loc.folder.path, &loc.remote_id, &action).await
            }
            .await;
            if let Err(e) = result {
                log::warn!("remote action {action:?} failed: {e}");
                ctx.events.emit(
                    events::SYNC_ERROR,
                    json!({ "accountId": loc.account_id, "error": e.to_string() }),
                );
            }
        });
        Ok(())
    }

    fn changed(&self, account_id: &str) {
        self.events.emit(events::MAIL_CHANGED, json!({ "accountId": account_id }));
    }

    pub fn mark_read(self: &Arc<Self>, id: &str, read: bool) -> AppResult<()> {
        self.db.set_read(id, read)?;
        self.push_remote(id, RemoteAction::MarkRead(read))
    }

    pub fn set_flagged(self: &Arc<Self>, id: &str, flagged: bool) -> AppResult<()> {
        self.db.set_flagged(id, flagged)?;
        self.push_remote(id, RemoteAction::Flag(flagged))
    }

    pub fn move_email(self: &Arc<Self>, id: &str, folder_id: &str) -> AppResult<()> {
        let loc = self.db.email_location(id)?;
        let target = self.db.get_folder(folder_id)?;
        if target.account_id != loc.account_id {
            return Err(AppError::Invalid("cannot move mail between accounts".into()));
        }
        if !target.is_local() {
            self.push_remote(id, RemoteAction::Move { to_path: target.path.clone() })?;
        }
        self.db.move_email(id, folder_id)?;
        self.changed(&loc.account_id);
        Ok(())
    }

    pub fn archive_email(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let loc = self.db.email_location(id)?;
        let archive = self.db.ensure_folder(&loc.account_id, FolderRole::Archive)?;
        if archive.id == loc.folder.id {
            return Ok(());
        }
        let archive_path = (!archive.is_local()).then(|| archive.path.clone());
        self.push_remote(id, RemoteAction::Archive { archive_path })?;
        self.db.move_email(id, &archive.id)?;
        self.changed(&loc.account_id);
        Ok(())
    }

    /// Moves to Trash; deleting from Trash (or Drafts) is permanent.
    pub fn delete_email(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let loc = self.db.email_location(id)?;
        if matches!(loc.folder.role, FolderRole::Trash | FolderRole::Drafts) {
            self.push_remote(id, RemoteAction::Delete)?;
            self.db.delete_email(id)?;
        } else {
            let trash = self.db.ensure_folder(&loc.account_id, FolderRole::Trash)?;
            let trash_path = (!trash.is_local()).then(|| trash.path.clone());
            self.push_remote(id, RemoteAction::Trash { trash_path })?;
            self.db.move_email(id, &trash.id)?;
        }
        self.changed(&loc.account_id);
        Ok(())
    }

    // ───────────────────────────── compose ──────────────────────────────

    pub fn build_message(account: &Account, out: &OutgoingEmail) -> AppResult<Message> {
        let recipients = |list: &[String]| -> AppResult<Vec<Mailbox>> {
            list.iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<Mailbox>().map_err(|e| AppError::Invalid(format!("invalid address '{s}': {e}"))))
                .collect()
        };
        let to = recipients(&out.to)?;
        if to.is_empty() {
            return Err(AppError::Invalid("add at least one recipient".into()));
        }

        let from = Mailbox::new(Some(account.display_name.clone()), account.email.parse()?);
        let mut builder = Message::builder().from(from).subject(out.subject.clone()).message_id(None);
        for m in to {
            builder = builder.to(m);
        }
        for m in recipients(&out.cc)? {
            builder = builder.cc(m);
        }
        for m in recipients(&out.bcc)? {
            builder = builder.bcc(m);
        }
        if let Some(irt) = out.in_reply_to.as_ref().filter(|s| !s.is_empty()) {
            builder = builder.in_reply_to(irt.clone()).references(irt.clone());
        }

        let html = out.body_html.as_ref().filter(|h| !h.trim().is_empty());
        let alternative = html.map(|h| MultiPart::alternative_plain_html(out.body_text.clone(), h.clone()));
        if out.attachments.is_empty() {
            return Ok(match alternative {
                Some(alt) => builder.multipart(alt)?,
                None => builder.singlepart(SinglePart::plain(out.body_text.clone()))?,
            });
        }
        let mut mixed = match alternative {
            Some(alt) => MultiPart::mixed().multipart(alt),
            None => MultiPart::mixed().singlepart(SinglePart::plain(out.body_text.clone())),
        };
        for a in &out.attachments {
            let data = base64::engine::general_purpose::STANDARD
                .decode(a.data_base64.as_bytes())
                .map_err(|e| AppError::Invalid(format!("attachment {}: {e}", a.filename)))?;
            let ct = ContentType::parse(&a.mime_type)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
            mixed = mixed.singlepart(Attachment::new(a.filename.clone()).body(data, ct));
        }
        Ok(builder.multipart(mixed)?)
    }

    pub async fn send_email(self: &Arc<Self>, out: OutgoingEmail) -> AppResult<String> {
        let account = self.db.get_account(&out.account_id)?;
        let message = Self::build_message(&account, &out)?;
        let creds = self.credentials(&account).await?;
        providers::outgoing_for(&account, &creds)?.send(&message).await?;

        // Keep a local copy in Sent.
        let raw = message.formatted();
        let sent = self.db.ensure_folder(&account.id, FolderRole::Sent)?;
        let parsed = mime::parse_message(&raw)?;
        let new = parsed.into_new_email(&account.id, &sent.id, &format!("local-{}", Uuid::new_v4()), true, false);
        let id = self.db.insert_email(&new)?.unwrap_or_default();

        // Plain SMTP servers don't file sent mail; Gmail/Graph do it themselves.
        if account.kind == AccountKind::Imap && !sent.is_local() {
            let ctx = Arc::clone(self);
            tokio::spawn(async move {
                let r = async {
                    let creds = ctx.credentials(&account).await?;
                    providers::incoming_for(&account, &creds)?.append(&sent.path, &raw).await
                }
                .await;
                if let Err(e) = r {
                    log::warn!("could not store sent copy: {e}");
                }
            });
        }
        if let Some(draft) = out.draft_id.as_deref() {
            let _ = self.db.delete_email(draft);
        }
        self.changed(&out.account_id);
        Ok(id)
    }

    /// Saves a draft locally (drafts may be incomplete, so no MIME is built).
    pub fn save_draft(&self, out: OutgoingEmail) -> AppResult<String> {
        let account = self.db.get_account(&out.account_id)?;
        let drafts = self.db.ensure_folder(&account.id, FolderRole::Drafts)?;
        if let Some(old) = out.draft_id.as_deref() {
            let _ = self.db.delete_email(old);
        }
        let addrs = |list: &[String]| -> Vec<Address> {
            list.iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| match s.parse::<Mailbox>() {
                    Ok(m) => Address { name: m.name, address: m.email.to_string() },
                    Err(_) => Address { name: None, address: s.to_string() },
                })
                .collect()
        };
        let mut attachments = Vec::new();
        for a in &out.attachments {
            attachments.push(NewAttachment {
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
                content_id: None,
                data: base64::engine::general_purpose::STANDARD
                    .decode(a.data_base64.as_bytes())
                    .map_err(|e| AppError::Invalid(format!("attachment {}: {e}", a.filename)))?,
            });
        }
        let new = NewEmail {
            account_id: account.id.clone(),
            folder_id: drafts.id,
            remote_id: format!("local-{}", Uuid::new_v4()),
            message_id: None,
            in_reply_to: out.in_reply_to.clone(),
            from: Address { name: Some(account.display_name.clone()), address: account.email.clone() },
            to: addrs(&out.to),
            cc: addrs(&out.cc),
            reply_to: vec![],
            subject: out.subject.clone(),
            snippet: mime::make_snippet(&out.body_text),
            body_text: Some(out.body_text.clone()),
            body_html: out.body_html.as_deref().map(mime::sanitize_html),
            date: chrono::Utc::now().timestamp(),
            is_read: true,
            is_flagged: false,
            attachments,
            logo_domain: None,
        };
        let id = self.db.insert_email(&new)?.ok_or_else(|| AppError::Protocol("draft not saved".into()))?;
        self.changed(&account.id);
        Ok(id)
    }
}
