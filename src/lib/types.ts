// Mirrors src-tauri/src/models.rs (serde camelCase).

export type AccountKind = "gmail" | "outlook" | "imap" | "pop3";
export type Security = "tls" | "start_tls" | "none";
export type FolderRole = "inbox" | "sent" | "drafts" | "spam" | "trash" | "archive" | "custom";
export type MailFilter = "all" | "unread" | "flagged";
export type MailSort = "newest" | "oldest" | "unread" | "flagged" | "sender" | "subject";

/** A user tag that can be put on any message (device-only). */
export interface Tag {
  id: string;
  name: string;
  /** `#rrggbb` */
  color: string;
  position: number;
  /** Messages carrying it in the queried account. */
  count: number;
}

export interface ServerConfig {
  host: string;
  port: number;
  security: Security;
}

export interface Account {
  id: string;
  kind: AccountKind;
  displayName: string;
  email: string;
  incoming: ServerConfig | null;
  outgoing: ServerConfig | null;
  username: string | null;
  pop3DownloadDays: number | null;
  color: string;
  createdAt: number;
}

export interface NewAccount {
  kind: AccountKind;
  displayName: string;
  email: string;
  incoming: ServerConfig | null;
  outgoing: ServerConfig | null;
  username: string | null;
  password: string | null;
  pop3DownloadDays: number | null;
}

export interface Folder {
  id: string;
  accountId: string;
  name: string;
  path: string;
  role: FolderRole;
  unreadCount: number;
  totalCount: number;
}

export interface Address {
  name: string | null;
  address: string;
}

export interface EmailSummary {
  id: string;
  accountId: string;
  folderId: string;
  from: Address;
  subject: string;
  snippet: string;
  /** Unix seconds */
  date: number;
  isRead: boolean;
  isFlagged: boolean;
  hasAttachments: boolean;
  /** Ids of the tags on this message. */
  tags: string[];
  hasNote: boolean;
  /** Sender domain that passed DMARC: its BIMI logo may be shown. */
  logoDomain: string | null;
}

/** Result of loading older messages of a folder from the server. */
export interface OlderPage {
  added: number;
  /** The server still has older ones. */
  hasMore: boolean;
}

export type OAuthProviderId = "google" | "microsoft";

/** A sign-in (OAuth) app as Settings shows it: the secret only masked. */
export interface OAuthAppInfo {
  provider: OAuthProviderId;
  clientId: string | null;
  /** e.g. `GOCSPX-••••••`; null when there is no secret. */
  secretHint: string | null;
  /** Where the app in use comes from; null when there is none. */
  source: "settings" | "environment" | "build" | null;
}

/** A sender's BIMI logo. */
export interface SenderLogo {
  /** Data URL of the BIMI logo, or of the site icon when there is no BIMI; null if neither. */
  svg: string | null;
  /** Backed by a Verified Mark Certificate checked up to a VMC root. */
  verified: boolean;
  /** Organisation that issued the certificate, e.g. "DigiCert, Inc.". */
  issuer: string | null;
}

export interface AttachmentMeta {
  id: string;
  emailId: string;
  filename: string;
  mimeType: string;
  size: number;
  contentId: string | null;
}

export interface Email extends EmailSummary {
  to: Address[];
  cc: Address[];
  replyTo: Address[];
  bodyHtml: string | null;
  bodyText: string | null;
  messageId: string | null;
  inReplyTo: string | null;
  attachments: AttachmentMeta[];
  /** Private note — stays on this device. */
  note: string | null;
}

export interface EmailQuery {
  accountId?: string | null;
  folderId?: string | null;
  /** Only messages with this tag (the folder is ignored then). */
  tagId?: string | null;
  filter?: MailFilter;
  sort?: MailSort;
  search?: string | null;
  limit?: number;
  offset?: number;
}

export interface EmailPage {
  items: EmailSummary[];
  total: number;
  hasMore: boolean;
}

export interface OutgoingAttachment {
  filename: string;
  mimeType: string;
  dataBase64: string;
}

export interface OutgoingEmail {
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyText: string;
  bodyHtml: string | null;
  attachments: OutgoingAttachment[];
  inReplyTo: string | null;
  draftId: string | null;
}

export interface ConnectionReport {
  incomingOk: boolean;
  outgoingOk: boolean;
  incomingError: string | null;
  outgoingError: string | null;
}

export interface DetectedSettings {
  provider: string | null;
  suggestedKind: AccountKind | null;
  imap: ServerConfig;
  pop3: ServerConfig;
  smtp: ServerConfig;
}

export type SyncState = "idle" | "syncing" | "error";

export interface SyncStatusEvent {
  accountId: string;
  state: SyncState;
  newMessages?: number;
  error?: string;
}

export interface MailNewEvent {
  accountId: string;
  count: number;
  latest: EmailSummary[];
}

export interface EventMap {
  "mail:new": MailNewEvent;
  "mail:changed": { accountId: string };
  "sync:status": SyncStatusEvent;
  "sync:error": { accountId: string; error: string };
}

export interface AppError {
  kind: string;
  message: string;
}
