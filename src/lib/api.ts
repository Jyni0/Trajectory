import type {
  Account,
  AccountKind,
  ConnectionReport,
  DetectedSettings,
  Email,
  EmailPage,
  EmailQuery,
  EmailSummary,
  EventMap,
  Folder,
  NewAccount,
  OAuthAppInfo,
  OlderPage,
  OAuthProviderId,
  OutgoingEmail,
  SenderLogo,
  Tag,
} from "./types";
import { createMockBackend } from "./mockBackend";

export type Unlisten = () => void;

/** Typed facade over the Rust IPC commands. */
export interface MailApi {
  listAccounts(): Promise<Account[]>;
  addAccount(account: NewAccount, verify?: boolean): Promise<Account>;
  addOAuthAccount(kind: AccountKind): Promise<Account>;
  /** Google / Microsoft sign-in apps (Settings → Sign-in apps). */
  oauthApps(): Promise<OAuthAppInfo[]>;
  /** Saves an app to the keychain; a null/empty secret keeps the saved one. */
  setOAuthApp(provider: OAuthProviderId, clientId: string, clientSecret: string | null): Promise<OAuthAppInfo>;
  clearOAuthApp(provider: OAuthProviderId): Promise<OAuthAppInfo>;
  removeAccount(id: string): Promise<void>;
  testConnection(account: NewAccount): Promise<ConnectionReport>;
  autodetect(email: string): Promise<DetectedSettings>;
  listFolders(accountId: string): Promise<Folder[]>;
  listEmails(query: EmailQuery): Promise<EmailPage>;
  getEmail(id: string): Promise<Email>;
  /** The conversation `id` belongs to, oldest first (always includes it). */
  getThread(id: string): Promise<EmailSummary[]>;
  /** Fetches the next batch of older messages of a folder from the server. */
  loadOlder(folderId: string): Promise<OlderPage>;
  /** BIMI logo of a DMARC-authenticated sender domain. */
  senderLogo(domain: string): Promise<SenderLogo>;
  listTags(accountId?: string | null): Promise<Tag[]>;
  createTag(name: string, color: string): Promise<Tag>;
  updateTag(id: string, name: string, color: string): Promise<Tag>;
  deleteTag(id: string): Promise<void>;
  reorderTags(ids: string[]): Promise<void>;
  setEmailTag(emailId: string, tagId: string, on: boolean): Promise<void>;
  setNote(emailId: string, note: string | null): Promise<void>;
  markRead(id: string, read: boolean): Promise<void>;
  setFlagged(id: string, flagged: boolean): Promise<void>;
  moveEmail(id: string, folderId: string): Promise<void>;
  archiveEmail(id: string): Promise<void>;
  deleteEmail(id: string): Promise<void>;
  sendEmail(email: OutgoingEmail): Promise<string>;
  saveDraft(email: OutgoingEmail): Promise<string>;
  syncNow(accountId?: string): Promise<void>;
  saveAttachment(id: string): Promise<string>;
  openExternal(url: string): Promise<void>;
  on<K extends keyof EventMap>(event: K, cb: (payload: EventMap[K]) => void): Promise<Unlisten>;
}

export const isTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function createTauriApi(): MailApi {
  // Loaded lazily so the browser/test build never touches Tauri internals.
  const core = import("@tauri-apps/api/core");
  const event = import("@tauri-apps/api/event");
  const invoke = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
    (await core).invoke<T>(cmd, args);

  return {
    listAccounts: () => invoke("list_accounts"),
    addAccount: (account, verify = true) => invoke("add_account", { account, verify }),
    addOAuthAccount: (kind) => invoke("add_oauth_account", { kind }),
    oauthApps: () => invoke("oauth_apps"),
    setOAuthApp: (provider, clientId, clientSecret) => invoke("set_oauth_app", { provider, clientId, clientSecret }),
    clearOAuthApp: (provider) => invoke("clear_oauth_app", { provider }),
    removeAccount: (id) => invoke("remove_account", { id }),
    testConnection: (account) => invoke("test_connection", { account }),
    autodetect: (email) => invoke("autodetect_settings", { email }),
    listFolders: (accountId) => invoke("list_folders", { accountId }),
    listEmails: (query) => invoke("list_emails", { query }),
    getEmail: (id) => invoke("get_email", { id }),
    getThread: (id) => invoke("get_thread", { id }),
    loadOlder: (folderId) => invoke("load_older", { folderId }),
    senderLogo: (domain) => invoke("sender_logo", { domain }),
    listTags: (accountId) => invoke("list_tags", { accountId: accountId ?? null }),
    createTag: (name, color) => invoke("create_tag", { name, color }),
    updateTag: (id, name, color) => invoke("update_tag", { id, name, color }),
    deleteTag: (id) => invoke("delete_tag", { id }),
    reorderTags: (ids) => invoke("reorder_tags", { ids }),
    setEmailTag: (emailId, tagId, on) => invoke("set_email_tag", { emailId, tagId, on }),
    setNote: (emailId, note) => invoke("set_note", { emailId, note }),
    markRead: (id, read) => invoke("mark_read", { id, read }),
    setFlagged: (id, flagged) => invoke("set_flagged", { id, flagged }),
    moveEmail: (id, folderId) => invoke("move_email", { id, folderId }),
    archiveEmail: (id) => invoke("archive_email", { id }),
    deleteEmail: (id) => invoke("delete_email", { id }),
    sendEmail: (email) => invoke("send_email", { email }),
    saveDraft: (email) => invoke("save_draft", { email }),
    syncNow: (accountId) => invoke("sync_now", { accountId: accountId ?? null }),
    saveAttachment: (id) => invoke("save_attachment", { id }),
    openExternal: (url) => invoke("open_external", { url }),
    on: async (name, cb) => (await event).listen(name, (e) => cb(e.payload as never)),
  };
}

let impl: MailApi = isTauri() ? createTauriApi() : createMockBackend({ latencyMs: 120 });

/** Swaps the backend (tests inject a fresh mock per test). */
export function setApi(next: MailApi) {
  impl = next;
}

export const api: MailApi = new Proxy({} as MailApi, {
  get: (_target, key) => impl[key as keyof MailApi],
});

export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  return "Something went wrong";
}
