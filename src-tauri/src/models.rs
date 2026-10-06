//! Domain types shared between the database, the sync engine and IPC.
//! All IPC-facing structs serialize as camelCase to match the TypeScript types
//! in `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Gmail,
    Outlook,
    Imap,
    Pop3,
}

impl AccountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountKind::Gmail => "gmail",
            AccountKind::Outlook => "outlook",
            AccountKind::Imap => "imap",
            AccountKind::Pop3 => "pop3",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "gmail" => AccountKind::Gmail,
            "outlook" => AccountKind::Outlook,
            "imap" => AccountKind::Imap,
            "pop3" => AccountKind::Pop3,
            _ => return None,
        })
    }

    pub fn is_oauth(&self) -> bool {
        matches!(self, AccountKind::Gmail | AccountKind::Outlook)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// Implicit TLS (IMAPS 993, POP3S 995, SMTPS 465).
    #[default]
    Tls,
    /// Plain connection upgraded via STARTTLS / STLS.
    StartTls,
    /// Unencrypted. Only meant for local test servers.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub display_name: String,
    pub email: String,
    pub incoming: Option<ServerConfig>,
    pub outgoing: Option<ServerConfig>,
    pub username: Option<String>,
    /// POP3 only: download messages not older than N days (None = everything).
    pub pop3_download_days: Option<u32>,
    pub color: String,
    pub created_at: i64,
}

/// Payload of `add_account` for password-based accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub kind: AccountKind,
    pub display_name: String,
    pub email: String,
    pub incoming: Option<ServerConfig>,
    pub outgoing: Option<ServerConfig>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub pop3_download_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Spam,
    Trash,
    Archive,
    Custom,
}

impl FolderRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Sent => "sent",
            FolderRole::Drafts => "drafts",
            FolderRole::Spam => "spam",
            FolderRole::Trash => "trash",
            FolderRole::Archive => "archive",
            FolderRole::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "inbox" => FolderRole::Inbox,
            "sent" => FolderRole::Sent,
            "drafts" => FolderRole::Drafts,
            "spam" => FolderRole::Spam,
            "trash" => FolderRole::Trash,
            "archive" => FolderRole::Archive,
            _ => FolderRole::Custom,
        }
    }

    /// Display order in the sidebar.
    pub fn sort_key(&self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Sent => 1,
            FolderRole::Drafts => 2,
            FolderRole::Archive => 3,
            FolderRole::Spam => 4,
            FolderRole::Trash => 5,
            FolderRole::Custom => 6,
        }
    }

    pub fn default_name(&self) -> &'static str {
        match self {
            FolderRole::Inbox => "Inbox",
            FolderRole::Sent => "Sent",
            FolderRole::Drafts => "Drafts",
            FolderRole::Spam => "Spam",
            FolderRole::Trash => "Trash",
            FolderRole::Archive => "Archive",
            FolderRole::Custom => "Folder",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    /// Remote identifier: IMAP mailbox path, Gmail label id, Graph folder id,
    /// or `local:<role>` for folders that exist only on this device.
    pub path: String,
    pub role: FolderRole,
    pub unread_count: i64,
    pub total_count: i64,
}

impl Folder {
    pub fn is_local(&self) -> bool {
        self.path.starts_with("local:")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub name: Option<String>,
    pub address: String,
}

impl Address {
    pub fn display(&self) -> String {
        match &self.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", self.address),
            _ => self.address.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmailSummary {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub from: Address,
    pub subject: String,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_flagged: bool,
    pub has_attachments: bool,
    /// Ids of the user's tags on this message.
    #[serde(default)]
    pub tags: Vec<String>,
    /// A private note is attached (the text itself comes with `Email`).
    #[serde(default)]
    pub has_note: bool,
    /// Sender domain that passed DMARC — the one whose BIMI logo may be shown.
    #[serde(default)]
    pub logo_domain: Option<String>,
}

/// A sender's BIMI logo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SenderLogo {
    /// The logo as a data URL (BIMI SVG, or the site icon when the domain has
    /// no BIMI), or none.
    pub svg: Option<String>,
    /// The logo comes from a Verified Mark Certificate that chains to a
    /// trusted VMC root, names this domain and embeds this very logo.
    pub verified: bool,
    /// Organisation of the certificate's issuer (e.g. "DigiCert, Inc.").
    pub issuer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub id: String,
    pub email_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Email {
    #[serde(flatten)]
    pub summary: EmailSummary,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachments: Vec<AttachmentMeta>,
    /// The user's private note on this message (device-only, never sent).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MailFilter {
    #[default]
    All,
    Unread,
    Flagged,
}

/// Order of the message list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MailSort {
    #[default]
    Newest,
    Oldest,
    /// Unread first, newest first within each group.
    Unread,
    /// Flagged first, newest first within each group.
    Flagged,
    Sender,
    Subject,
}

impl MailSort {
    pub fn order_by(self) -> &'static str {
        match self {
            MailSort::Newest => "date DESC, id",
            MailSort::Oldest => "date ASC, id",
            MailSort::Unread => "is_read ASC, date DESC, id",
            MailSort::Flagged => "is_flagged DESC, date DESC, id",
            MailSort::Sender => "lower(coalesce(nullif(from_name, ''), from_address)) ASC, date DESC, id",
            MailSort::Subject => "lower(subject) ASC, date DESC, id",
        }
    }
}

/// A user tag ("note label") that can be put on any message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Manual order in the sidebar (0 = top).
    pub position: i64,
    /// Messages carrying the tag (in the queried account, or everywhere).
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmailQuery {
    pub account_id: Option<String>,
    pub folder_id: Option<String>,
    /// Only messages with this tag (folder is ignored then).
    #[serde(default)]
    pub tag_id: Option<String>,
    #[serde(default)]
    pub filter: MailFilter,
    #[serde(default)]
    pub sort: MailSort,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmailPage {
    pub items: Vec<EmailSummary>,
    pub total: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingEmail {
    pub account_id: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachment>,
    pub in_reply_to: Option<String>,
    /// When sending/saving an existing draft, the draft gets replaced.
    pub draft_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReport {
    pub incoming_ok: bool,
    pub outgoing_ok: bool,
    pub incoming_error: Option<String>,
    pub outgoing_error: Option<String>,
}

impl ConnectionReport {
    pub fn ok(&self) -> bool {
        self.incoming_ok && self.outgoing_ok
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSettings {
    pub provider: Option<String>,
    /// Suggests using the OAuth flow instead of passwords (gmail/outlook).
    pub suggested_kind: Option<AccountKind>,
    pub imap: ServerConfig,
    pub pop3: ServerConfig,
    pub smtp: ServerConfig,
}

/// Result of loading older messages of a folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OlderPage {
    /// Messages added to the folder.
    pub added: u32,
    /// The server has still older ones.
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub account_id: String,
    pub new_messages: u32,
    pub folders: u32,
}
