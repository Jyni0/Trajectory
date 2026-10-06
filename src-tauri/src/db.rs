//! SQLite cache for accounts, folders, emails and attachments.
//!
//! The connection is guarded by a `std::sync::Mutex`: every operation is a
//! short local query, so blocking the async worker for a few microseconds is
//! cheaper than shuttling work to a blocking pool.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::*;

const SCHEMA_VERSION: i32 = 5;

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id                  TEXT PRIMARY KEY,
    kind                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    email               TEXT NOT NULL,
    incoming_json       TEXT,
    outgoing_json       TEXT,
    username            TEXT,
    pop3_download_days  INTEGER,
    color               TEXT NOT NULL,
    created_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    path        TEXT NOT NULL,
    role        TEXT NOT NULL,
    UNIQUE (account_id, path)
);

CREATE TABLE IF NOT EXISTS emails (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id       TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    remote_id       TEXT NOT NULL,
    message_id      TEXT,
    in_reply_to     TEXT,
    from_name       TEXT,
    from_address    TEXT NOT NULL,
    to_json         TEXT NOT NULL,
    cc_json         TEXT NOT NULL,
    reply_to_json   TEXT NOT NULL,
    subject         TEXT NOT NULL,
    snippet         TEXT NOT NULL,
    body_text       TEXT,
    body_html       TEXT,
    date            INTEGER NOT NULL,
    is_read         INTEGER NOT NULL DEFAULT 0,
    is_flagged      INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    UNIQUE (folder_id, remote_id)
);
CREATE INDEX IF NOT EXISTS idx_emails_folder_date ON emails(folder_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_emails_account_date ON emails(account_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_emails_message_id ON emails(folder_id, message_id);

CREATE TABLE IF NOT EXISTS attachments (
    id          TEXT PRIMARY KEY,
    email_id    TEXT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    filename    TEXT NOT NULL,
    mime_type   TEXT NOT NULL,
    size        INTEGER NOT NULL,
    content_id  TEXT,
    data        BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachments_email ON attachments(email_id);

CREATE TABLE IF NOT EXISTS sync_state (
    folder_id   TEXT PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE,
    state       TEXT NOT NULL
);
"#;

/// Tags and notes: the user's own organisation of mail. Device-only — never
/// pushed to the server, so they work the same for every account kind.
const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS tags (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    color       TEXT NOT NULL,
    position    INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name ON tags(lower(name));

CREATE TABLE IF NOT EXISTS email_tags (
    email_id    TEXT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    tag_id      TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (email_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_email_tags_tag ON email_tags(tag_id);

ALTER TABLE emails ADD COLUMN note TEXT;
"#;

/// The sanitizer used to keep the text of `<title>` (stray text above every
/// HTML mail) and drop `<style>`. Forgetting the sync cursors makes the next
/// sync fetch the recent mail again; `insert_email` then refreshes the bodies
/// of messages it already has (tags, notes, read/flag state are kept).
const MIGRATION_V3: &str = "DELETE FROM sync_state;";

/// BIMI sender logos: the DMARC-authenticated sender domain per message, and
/// a cache of each domain's logo. Cursors are forgotten again so recent mail
/// is re-read and gets its domain.
const MIGRATION_V4: &str = r#"
ALTER TABLE emails ADD COLUMN logo_domain TEXT;
CREATE TABLE IF NOT EXISTS sender_logos (
    domain      TEXT PRIMARY KEY,
    svg         TEXT,
    verified    INTEGER NOT NULL DEFAULT 0,
    issuer      TEXT,
    fetched_at  INTEGER NOT NULL
);
DELETE FROM sync_state;
"#;

/// Longest tag name / note accepted.
pub const TAG_NAME_MAX: usize = 40;
pub const NOTE_MAX: usize = 10_000;

/// Summary columns, in the order `summary_from_row` reads them (0..=13).
const SUMMARY_COLS: &str = "id, account_id, folder_id, from_name, from_address, subject, snippet, date,
    is_read, is_flagged, has_attachments,
    (SELECT group_concat(tag_id) FROM email_tags t WHERE t.email_id = emails.id),
    coalesce(note, '') <> '', logo_domain";

/// Email ready to be written into the cache (produced by the MIME parser or
/// by the composer for drafts / sent copies).
#[derive(Debug, Clone, Default)]
pub struct NewEmail {
    pub account_id: String,
    pub folder_id: String,
    pub remote_id: String,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub subject: String,
    pub snippet: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub date: i64,
    pub is_read: bool,
    pub is_flagged: bool,
    pub attachments: Vec<NewAttachment>,
    /// Sender domain that passed DMARC, if any.
    pub logo_domain: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewAttachment {
    pub filename: String,
    pub mime_type: String,
    pub content_id: Option<String>,
    pub data: Vec<u8>,
}

/// Where an email lives remotely — needed to replay user actions on the server.
#[derive(Debug, Clone)]
pub struct EmailLocation {
    pub email_id: String,
    pub account_id: String,
    pub folder: Folder,
    pub remote_id: String,
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Deleted mail is overwritten with zeros instead of lingering in free pages.
        conn.pragma_update(None, "secure_delete", "ON")?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(MIGRATION_V1)?;
        }
        if version < 2 {
            conn.execute_batch(MIGRATION_V2)?;
        }
        if (1..3).contains(&version) {
            conn.execute_batch(MIGRATION_V3)?;
        }
        if version < 4 {
            conn.execute_batch(MIGRATION_V4)?;
        }
        if (4..5).contains(&version) {
            // Site icons are now the fallback: forget the cached "no logo"s.
            conn.execute_batch("DELETE FROM sender_logos;")?;
        }
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-query; the
        // connection itself is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ───────────────────────────── accounts ─────────────────────────────

    pub fn insert_account(&self, a: &Account) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO accounts (id, kind, display_name, email, incoming_json, outgoing_json,
                                   username, pop3_download_days, color, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                a.id,
                a.kind.as_str(),
                a.display_name,
                a.email,
                a.incoming.as_ref().map(serde_json::to_string).transpose()?,
                a.outgoing.as_ref().map(serde_json::to_string).transpose()?,
                a.username,
                a.pop3_download_days,
                a.color,
                a.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> AppResult<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM accounts ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], account_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn get_account(&self, id: &str) -> AppResult<Account> {
        self.conn()
            .query_row("SELECT * FROM accounts WHERE id = ?1", [id], account_from_row)
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("account {id}")))
    }

    pub fn find_account_by_email(&self, email: &str) -> AppResult<Option<Account>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT * FROM accounts WHERE lower(email) = lower(?1)",
                [email],
                account_from_row,
            )
            .optional()?)
    }

    pub fn delete_account(&self, id: &str) -> AppResult<()> {
        let n = self.conn().execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("account {id}")));
        }
        Ok(())
    }

    // ───────────────────────────── folders ──────────────────────────────

    pub fn list_folders(&self, account_id: &str) -> AppResult<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.account_id, f.name, f.path, f.role,
                    (SELECT COUNT(*) FROM emails e WHERE e.folder_id = f.id AND e.is_read = 0),
                    (SELECT COUNT(*) FROM emails e WHERE e.folder_id = f.id)
             FROM folders f WHERE f.account_id = ?1",
        )?;
        let mut folders: Vec<Folder> =
            stmt.query_map([account_id], folder_from_row)?.collect::<Result<_, _>>()?;
        folders.sort_by(|a, b| {
            a.role
                .sort_key()
                .cmp(&b.role.sort_key())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(folders)
    }

    pub fn get_folder(&self, id: &str) -> AppResult<Folder> {
        self.conn()
            .query_row(
                "SELECT f.id, f.account_id, f.name, f.path, f.role,
                        (SELECT COUNT(*) FROM emails e WHERE e.folder_id = f.id AND e.is_read = 0),
                        (SELECT COUNT(*) FROM emails e WHERE e.folder_id = f.id)
                 FROM folders f WHERE f.id = ?1",
                [id],
                folder_from_row,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("folder {id}")))
    }

    pub fn find_folder_by_role(&self, account_id: &str, role: FolderRole) -> AppResult<Option<Folder>> {
        let id: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND role = ?2 ORDER BY path LIKE 'local:%' LIMIT 1",
                params![account_id, role.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| self.get_folder(&id)).transpose()
    }

    /// Creates a device-only folder for `role` unless one with that role exists.
    pub fn ensure_folder(&self, account_id: &str, role: FolderRole) -> AppResult<Folder> {
        if let Some(f) = self.find_folder_by_role(account_id, role)? {
            return Ok(f);
        }
        let id = Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT INTO folders (id, account_id, name, path, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, account_id, role.default_name(), format!("local:{}", role.as_str()), role.as_str()],
        )?;
        self.get_folder(&id)
    }

    /// Upserts a folder discovered on the server. A device-only placeholder with
    /// the same role is promoted to the remote folder so cached mail is kept.
    pub fn upsert_remote_folder(
        &self,
        account_id: &str,
        name: &str,
        path: &str,
        role: FolderRole,
    ) -> AppResult<Folder> {
        let existing: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND path = ?2",
                params![account_id, path],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn()
                .execute("UPDATE folders SET name = ?1 WHERE id = ?2", params![name, id])?;
            return self.get_folder(&id);
        }

        let mut role = role;
        if role != FolderRole::Custom {
            if let Some(f) = self.find_folder_by_role(account_id, role)? {
                if f.is_local() {
                    self.conn().execute(
                        "UPDATE folders SET name = ?1, path = ?2 WHERE id = ?3",
                        params![name, path, f.id],
                    )?;
                    return self.get_folder(&f.id);
                }
                // A remote folder already owns this role; keep the new one as a label.
                role = FolderRole::Custom;
            }
        }

        let id = Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT INTO folders (id, account_id, name, path, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, account_id, name, path, role.as_str()],
        )?;
        self.get_folder(&id)
    }

    // ───────────────────────────── emails ───────────────────────────────

    /// Inserts an email. Returns `None` when it is already cached: either the
    /// same remote id, or the same Message-ID in the same folder (in which case
    /// the remote id is refreshed — e.g. after a server-side move/append). A
    /// cached copy gets the freshly parsed body; user state stays as it is.
    pub fn insert_email(&self, e: &NewEmail) -> AppResult<Option<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;

        let n = tx.execute(
            "UPDATE emails SET body_html = ?3, body_text = ?4, snippet = ?5, logo_domain = ?6
             WHERE folder_id = ?1 AND remote_id = ?2",
            params![e.folder_id, e.remote_id, e.body_html, e.body_text, e.snippet, e.logo_domain],
        )?;
        if n > 0 {
            tx.commit()?;
            return Ok(None);
        }
        if let Some(mid) = &e.message_id {
            let n = tx.execute(
                "UPDATE emails SET remote_id = ?1, body_html = ?4, body_text = ?5, snippet = ?6, logo_domain = ?7
                 WHERE folder_id = ?2 AND message_id = ?3",
                params![e.remote_id, e.folder_id, mid, e.body_html, e.body_text, e.snippet, e.logo_domain],
            )?;
            if n > 0 {
                tx.commit()?;
                return Ok(None);
            }
        }

        let id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO emails (id, account_id, folder_id, remote_id, message_id, in_reply_to,
                from_name, from_address, to_json, cc_json, reply_to_json, subject, snippet,
                body_text, body_html, date, is_read, is_flagged, has_attachments, logo_domain)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                id,
                e.account_id,
                e.folder_id,
                e.remote_id,
                e.message_id,
                e.in_reply_to,
                e.from.name,
                e.from.address,
                serde_json::to_string(&e.to)?,
                serde_json::to_string(&e.cc)?,
                serde_json::to_string(&e.reply_to)?,
                e.subject,
                e.snippet,
                e.body_text,
                e.body_html,
                e.date,
                e.is_read,
                e.is_flagged,
                // Inline images (content-id) are not shown as attachments.
                e.attachments.iter().any(|a| a.content_id.is_none()),
                e.logo_domain,
            ],
        )?;
        for a in &e.attachments {
            tx.execute(
                "INSERT INTO attachments (id, email_id, filename, mime_type, size, content_id, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    a.filename,
                    a.mime_type,
                    a.data.len() as i64,
                    a.content_id,
                    a.data
                ],
            )?;
        }
        tx.commit()?;
        Ok(Some(id))
    }

    pub fn list_emails(&self, q: &EmailQuery) -> AppResult<EmailPage> {
        let mut where_sql = vec!["1 = 1".to_string()];
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(tag) = &q.tag_id {
            args.push(Box::new(tag.clone()));
            where_sql.push(format!("id IN (SELECT email_id FROM email_tags WHERE tag_id = ?{})", args.len()));
            if let Some(account) = &q.account_id {
                args.push(Box::new(account.clone()));
                where_sql.push(format!("account_id = ?{}", args.len()));
            }
        } else if let Some(folder) = &q.folder_id {
            args.push(Box::new(folder.clone()));
            where_sql.push(format!("folder_id = ?{}", args.len()));
        } else if let Some(account) = &q.account_id {
            args.push(Box::new(account.clone()));
            where_sql.push(format!("account_id = ?{}", args.len()));
        }
        match q.filter {
            MailFilter::All => {}
            MailFilter::Unread => where_sql.push("is_read = 0".into()),
            MailFilter::Flagged => where_sql.push("is_flagged = 1".into()),
        }
        if let Some(term) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            args.push(Box::new(format!("%{escaped}%")));
            let n = args.len();
            where_sql.push(format!(
                "(subject LIKE ?{n} ESCAPE '\\' OR from_address LIKE ?{n} ESCAPE '\\'
                  OR from_name LIKE ?{n} ESCAPE '\\' OR snippet LIKE ?{n} ESCAPE '\\'
                  OR body_text LIKE ?{n} ESCAPE '\\' OR note LIKE ?{n} ESCAPE '\\')"
            ));
        }
        let where_clause = where_sql.join(" AND ");
        let limit = q.limit.unwrap_or(50).clamp(1, 500) as i64;
        let offset = q.offset.unwrap_or(0) as i64;

        let conn = self.conn();
        let arg_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM emails WHERE {where_clause}"),
            arg_refs.as_slice(),
            |r| r.get(0),
        )?;
        let order = q.sort.order_by();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM emails WHERE {where_clause}
             ORDER BY {order} LIMIT {limit} OFFSET {offset}"
        ))?;
        let items: Vec<EmailSummary> = stmt
            .query_map(arg_refs.as_slice(), summary_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(EmailPage { has_more: offset + (items.len() as i64) < total, items, total })
    }

    pub fn get_email(&self, id: &str) -> AppResult<Email> {
        let conn = self.conn();
        let mut email = conn
            .query_row(
                &format!(
                    "SELECT {SUMMARY_COLS},
                            to_json, cc_json, reply_to_json, body_html, body_text, message_id, in_reply_to, note
                     FROM emails WHERE id = ?1"
                ),
                [id],
                |r| {
                    let to: String = r.get(14)?;
                    let cc: String = r.get(15)?;
                    let reply_to: String = r.get(16)?;
                    Ok(Email {
                        summary: summary_from_row(r)?,
                        to: serde_json::from_str(&to).unwrap_or_default(),
                        cc: serde_json::from_str(&cc).unwrap_or_default(),
                        reply_to: serde_json::from_str(&reply_to).unwrap_or_default(),
                        body_html: r.get(17)?,
                        body_text: r.get(18)?,
                        message_id: r.get(19)?,
                        in_reply_to: r.get(20)?,
                        attachments: vec![],
                        note: r.get(21)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("email {id}")))?;

        let mut stmt = conn.prepare(
            "SELECT id, email_id, filename, mime_type, size, content_id
             FROM attachments WHERE email_id = ?1 ORDER BY rowid",
        )?;
        email.attachments = stmt
            .query_map([id], attachment_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(email)
    }

    /// The conversation an email belongs to, oldest first: every message of the
    /// account linked to it through Message-ID / In-Reply-To, in any folder
    /// but Trash, Spam and Drafts (the email itself is always there). A message
    /// stored in two folders (Inbox + a label, say) appears once.
    pub fn thread(&self, id: &str) -> AppResult<Vec<EmailSummary>> {
        /// Bounds for pathological chains.
        const MAX_MESSAGES: usize = 200;
        const MAX_ROUNDS: usize = 20;
        let conn = self.conn();
        let (account, mid, irt): (String, Option<String>, Option<String>) = conn
            .query_row("SELECT account_id, message_id, in_reply_to FROM emails WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("email {id}")))?;

        let mut keys: std::collections::BTreeSet<String> = [mid, irt].into_iter().flatten().map(|k| msg_key(&k)).filter(|k| !k.is_empty()).collect();
        let mut members: std::collections::BTreeSet<String> = [id.to_string()].into();
        for _ in 0..MAX_ROUNDS {
            if keys.is_empty() || members.len() >= MAX_MESSAGES {
                break;
            }
            let list: Vec<&String> = keys.iter().collect();
            let marks = vec!["?"; list.len()].join(",");
            let sql = format!(
                "SELECT id, message_id, in_reply_to FROM emails
                 WHERE account_id = ? AND (lower(trim(message_id, '<> ')) IN ({marks}) OR lower(trim(in_reply_to, '<> ')) IN ({marks}))
                   AND folder_id NOT IN (SELECT id FROM folders WHERE role IN ('trash', 'spam', 'drafts'))
                 LIMIT {MAX_MESSAGES}"
            );
            let mut args: Vec<&dyn rusqlite::ToSql> = vec![&account];
            args.extend(list.iter().map(|k| *k as &dyn rusqlite::ToSql));
            args.extend(list.iter().map(|k| *k as &dyn rusqlite::ToSql));
            let mut stmt = conn.prepare(&sql)?;
            let rows: Vec<(String, Option<String>, Option<String>)> =
                stmt.query_map(args.as_slice(), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            let before = (keys.len(), members.len());
            for (eid, m, i) in rows {
                members.insert(eid);
                keys.extend([m, i].into_iter().flatten().map(|k| msg_key(&k)).filter(|k| !k.is_empty()));
            }
            if (keys.len(), members.len()) == before {
                break;
            }
        }

        let list: Vec<&String> = members.iter().collect();
        let marks = vec!["?"; list.len()].join(",");
        let mut stmt = conn.prepare(&format!("SELECT {SUMMARY_COLS}, message_id FROM emails WHERE id IN ({marks}) ORDER BY date, rowid"))?;
        let args: Vec<&dyn rusqlite::ToSql> = list.iter().map(|k| *k as &dyn rusqlite::ToSql).collect();
        let rows: Vec<(EmailSummary, Option<String>)> =
            stmt.query_map(args.as_slice(), |r| Ok((summary_from_row(r)?, r.get(14)?)))?.collect::<Result<_, _>>()?;
        // One copy per Message-ID; the opened email wins over its duplicates.
        let mut seen = std::collections::HashMap::<String, usize>::new();
        let mut out: Vec<EmailSummary> = Vec::with_capacity(rows.len());
        for (summary, mid) in rows {
            let key = mid.as_deref().map(msg_key).filter(|k| !k.is_empty());
            match key.as_ref().and_then(|k| seen.get(k)) {
                Some(&at) => {
                    if summary.id == id {
                        out[at] = summary;
                    }
                }
                None => {
                    if let Some(k) = key {
                        seen.insert(k, out.len());
                    }
                    out.push(summary);
                }
            }
        }
        Ok(out)
    }

    pub fn get_attachment(&self, id: &str) -> AppResult<(AttachmentMeta, Vec<u8>)> {
        self.conn()
            .query_row(
                "SELECT id, email_id, filename, mime_type, size, content_id, data
                 FROM attachments WHERE id = ?1",
                [id],
                |r| Ok((attachment_from_row(r)?, r.get(6)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("attachment {id}")))
    }

    /// Inline (content-id) parts of an email, used to resolve `cid:` image URLs.
    pub fn inline_parts(&self, email_id: &str) -> AppResult<Vec<(String, String, Vec<u8>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT content_id, mime_type, data FROM attachments
             WHERE email_id = ?1 AND content_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([email_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn email_location(&self, id: &str) -> AppResult<EmailLocation> {
        let (account_id, folder_id, remote_id): (String, String, String) = self
            .conn()
            .query_row(
                "SELECT account_id, folder_id, remote_id FROM emails WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("email {id}")))?;
        Ok(EmailLocation {
            email_id: id.to_string(),
            account_id,
            folder: self.get_folder(&folder_id)?,
            remote_id,
        })
    }

    pub fn set_read(&self, id: &str, read: bool) -> AppResult<()> {
        self.update_one("UPDATE emails SET is_read = ?1 WHERE id = ?2", read, id)
    }

    pub fn set_flagged(&self, id: &str, flagged: bool) -> AppResult<()> {
        self.update_one("UPDATE emails SET is_flagged = ?1 WHERE id = ?2", flagged, id)
    }

    pub fn move_email(&self, id: &str, folder_id: &str) -> AppResult<()> {
        let n = self.conn().execute(
            "UPDATE emails SET folder_id = ?1 WHERE id = ?2",
            params![folder_id, id],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(format!("email {id}")));
        }
        Ok(())
    }

    pub fn delete_email(&self, id: &str) -> AppResult<()> {
        let n = self.conn().execute("DELETE FROM emails WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("email {id}")));
        }
        Ok(())
    }

    fn update_one(&self, sql: &str, value: bool, id: &str) -> AppResult<()> {
        let n = self.conn().execute(sql, params![value, id])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("email {id}")));
        }
        Ok(())
    }

    pub fn unread_total(&self) -> AppResult<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM emails e JOIN folders f ON f.id = e.folder_id
             WHERE e.is_read = 0 AND f.role = 'inbox'",
            [],
            |r| r.get(0),
        )?)
    }

    // ─────────────────────────── tags & notes ───────────────────────────

    /// All tags in their manual order; `count` is limited to one account when given.
    pub fn list_tags(&self, account_id: Option<&str>) -> AppResult<Vec<Tag>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.position,
                    (SELECT COUNT(*) FROM email_tags et JOIN emails e ON e.id = et.email_id
                     WHERE et.tag_id = t.id AND (?1 IS NULL OR e.account_id = ?1))
             FROM tags t ORDER BY t.position, lower(t.name)",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(Tag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, position: r.get(3)?, count: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get_tag(&self, id: &str) -> AppResult<Tag> {
        self.list_tags(None)?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| AppError::NotFound(format!("tag {id}")))
    }

    /// Creates a tag at the bottom of the list. Names are unique, ignoring case.
    pub fn create_tag(&self, name: &str, color: &str) -> AppResult<Tag> {
        let name = tag_name(name)?;
        let color = tag_color(color)?;
        let id = Uuid::new_v4().to_string();
        {
            let conn = self.conn();
            let position: i64 = conn.query_row("SELECT coalesce(MAX(position) + 1, 0) FROM tags", [], |r| r.get(0))?;
            conn.execute(
                "INSERT INTO tags (id, name, color, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, color, position, chrono::Utc::now().timestamp()],
            )
            .map_err(|e| unique_name(e, &name))?;
        }
        self.get_tag(&id)
    }

    pub fn update_tag(&self, id: &str, name: &str, color: &str) -> AppResult<Tag> {
        let name = tag_name(name)?;
        let color = tag_color(color)?;
        let n = self
            .conn()
            .execute("UPDATE tags SET name = ?1, color = ?2 WHERE id = ?3", params![name, color, id])
            .map_err(|e| unique_name(e, &name))?;
        if n == 0 {
            return Err(AppError::NotFound(format!("tag {id}")));
        }
        self.get_tag(id)
    }

    /// Deletes a tag; it disappears from every message (the messages stay).
    pub fn delete_tag(&self, id: &str) -> AppResult<()> {
        let n = self.conn().execute("DELETE FROM tags WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("tag {id}")));
        }
        Ok(())
    }

    /// Stores a new manual order. Tags missing from `ids` keep their relative
    /// order after the listed ones.
    pub fn reorder_tags(&self, ids: &[String]) -> AppResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let rest: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM tags ORDER BY position, lower(name)")?;
            let all: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            all.into_iter().filter(|id| !ids.contains(id)).collect()
        };
        for (i, id) in ids.iter().chain(rest.iter()).enumerate() {
            tx.execute("UPDATE tags SET position = ?1 WHERE id = ?2", params![i as i64, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Puts a tag on a message or takes it off.
    pub fn set_email_tag(&self, email_id: &str, tag_id: &str, on: bool) -> AppResult<()> {
        let conn = self.conn();
        let exists = |sql: &str, id: &str| -> AppResult<bool> {
            Ok(conn.query_row(sql, [id], |_| Ok(())).optional()?.is_some())
        };
        if !exists("SELECT 1 FROM emails WHERE id = ?1", email_id)? {
            return Err(AppError::NotFound(format!("email {email_id}")));
        }
        if !exists("SELECT 1 FROM tags WHERE id = ?1", tag_id)? {
            return Err(AppError::NotFound(format!("tag {tag_id}")));
        }
        if on {
            conn.execute(
                "INSERT OR IGNORE INTO email_tags (email_id, tag_id) VALUES (?1, ?2)",
                params![email_id, tag_id],
            )?;
        } else {
            conn.execute("DELETE FROM email_tags WHERE email_id = ?1 AND tag_id = ?2", params![email_id, tag_id])?;
        }
        Ok(())
    }

    /// Saves the private note of a message; blank removes it.
    pub fn set_note(&self, email_id: &str, note: Option<&str>) -> AppResult<()> {
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        if note.is_some_and(|n| n.chars().count() > NOTE_MAX) {
            return Err(AppError::Invalid(format!("a note can be at most {NOTE_MAX} characters")));
        }
        let n = self.conn().execute("UPDATE emails SET note = ?1 WHERE id = ?2", params![note, email_id])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("email {email_id}")));
        }
        Ok(())
    }

    // ─────────────────────────── sender logos ───────────────────────────

    /// Cached logo of a domain and when it was looked up.
    pub fn cached_logo(&self, domain: &str) -> AppResult<Option<(SenderLogo, i64)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT svg, verified, issuer, fetched_at FROM sender_logos WHERE domain = ?1",
                [domain],
                |r| Ok((SenderLogo { svg: r.get(0)?, verified: r.get(1)?, issuer: r.get(2)? }, r.get(3)?)),
            )
            .optional()?)
    }

    pub fn store_logo(&self, domain: &str, logo: &SenderLogo, fetched_at: i64) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO sender_logos (domain, svg, verified, issuer, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(domain) DO UPDATE SET svg = excluded.svg, verified = excluded.verified,
                 issuer = excluded.issuer, fetched_at = excluded.fetched_at",
            params![domain, logo.svg, logo.verified, logo.issuer, fetched_at],
        )?;
        Ok(())
    }

    // ──────────────────────────── sync state ────────────────────────────

    /// Remote ids of the messages a folder already holds.
    pub fn remote_ids(&self, folder_id: &str) -> AppResult<std::collections::HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT remote_id FROM emails WHERE folder_id = ?1")?;
        let ids = stmt.query_map([folder_id], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
        Ok(ids)
    }

    pub fn get_sync_state(&self, folder_id: &str) -> AppResult<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT state FROM sync_state WHERE folder_id = ?1", [folder_id], |r| r.get(0))
            .optional()?)
    }

    pub fn set_sync_state(&self, folder_id: &str, state: &str) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO sync_state (folder_id, state) VALUES (?1, ?2)
             ON CONFLICT(folder_id) DO UPDATE SET state = excluded.state",
            params![folder_id, state],
        )?;
        Ok(())
    }
}

fn tag_name(name: &str) -> AppResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(AppError::Invalid("a tag needs a name".into()));
    }
    if name.chars().count() > TAG_NAME_MAX {
        return Err(AppError::Invalid(format!("a tag name can be at most {TAG_NAME_MAX} characters")));
    }
    Ok(name)
}

/// Tag colours are plain `#rrggbb` — they end up in style attributes.
fn tag_color(color: &str) -> AppResult<String> {
    let c = color.trim();
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if !ok {
        return Err(AppError::Invalid(format!("'{c}' is not a #rrggbb colour")));
    }
    Ok(c.to_ascii_lowercase())
}

fn unique_name(e: rusqlite::Error, name: &str) -> AppError {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            AppError::Invalid(format!("a tag named '{name}' already exists"))
        }
        e => e.into(),
    }
}

fn account_from_row(r: &Row<'_>) -> rusqlite::Result<Account> {
    let kind: String = r.get("kind")?;
    let incoming: Option<String> = r.get("incoming_json")?;
    let outgoing: Option<String> = r.get("outgoing_json")?;
    Ok(Account {
        id: r.get("id")?,
        kind: AccountKind::parse(&kind).unwrap_or(AccountKind::Imap),
        display_name: r.get("display_name")?,
        email: r.get("email")?,
        incoming: incoming.and_then(|s| serde_json::from_str(&s).ok()),
        outgoing: outgoing.and_then(|s| serde_json::from_str(&s).ok()),
        username: r.get("username")?,
        pop3_download_days: r.get("pop3_download_days")?,
        color: r.get("color")?,
        created_at: r.get("created_at")?,
    })
}

fn folder_from_row(r: &Row<'_>) -> rusqlite::Result<Folder> {
    let role = FolderRole::parse(&r.get::<_, String>(4)?);
    let name: String = r.get(2)?;
    Ok(Folder {
        id: r.get(0)?,
        account_id: r.get(1)?,
        // Servers name system folders as they like ("INBOX", "[Gmail]/Sent
        // Mail", "Gesendet"…): those always show under the app's own name.
        name: if role == FolderRole::Custom { name } else { role.default_name().to_string() },
        path: r.get(3)?,
        role,
        unread_count: r.get(5)?,
        total_count: r.get(6)?,
    })
}

/// A Message-ID as compared: no angle brackets, lower case.
fn msg_key(id: &str) -> String {
    id.trim().trim_matches(|c| c == '<' || c == '>' || c == ' ').to_ascii_lowercase()
}

fn summary_from_row(r: &Row<'_>) -> rusqlite::Result<EmailSummary> {
    Ok(EmailSummary {
        id: r.get(0)?,
        account_id: r.get(1)?,
        folder_id: r.get(2)?,
        from: Address { name: r.get(3)?, address: r.get(4)? },
        subject: r.get(5)?,
        snippet: r.get(6)?,
        date: r.get(7)?,
        is_read: r.get(8)?,
        is_flagged: r.get(9)?,
        has_attachments: r.get(10)?,
        tags: r
            .get::<_, Option<String>>(11)?
            .map(|s| s.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
        has_note: r.get(12)?,
        logo_domain: r.get(13)?,
    })
}

fn attachment_from_row(r: &Row<'_>) -> rusqlite::Result<AttachmentMeta> {
    Ok(AttachmentMeta {
        id: r.get(0)?,
        email_id: r.get(1)?,
        filename: r.get(2)?,
        mime_type: r.get(3)?,
        size: r.get(4)?,
        content_id: r.get(5)?,
    })
}
