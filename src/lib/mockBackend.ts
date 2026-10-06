/**
 * In-memory implementation of the Rust core. Used when the UI runs outside
 * Tauri (browser dev, Vitest, Playwright E2E). It mirrors the backend's
 * semantics: system folders, delete → Trash → permanent, local drafts, etc.
 */
import type { MailApi, Unlisten } from "./api";
import type {
  Account,
  AccountKind,
  Address,
  DetectedSettings,
  Email,
  EmailSummary,
  EventMap,
  Folder,
  FolderRole,
  NewAccount,
  OutgoingEmail,
  SenderLogo,
  OAuthAppInfo,
  OAuthProviderId,
  ServerConfig,
  Tag,
} from "./types";

export interface MockOptions {
  latencyMs?: number;
  /** Pre-create a demo account with mail. */
  withDemoAccount?: boolean;
  /** Also add a second (Gmail) demo mailbox — the browser preview uses it. */
  secondDemoAccount?: boolean;
  /** Messages generated for each new account. */
  seedCount?: number;
  /** Add a four-message support conversation to the demo account (preview). */
  demoThread?: boolean;
}

export interface MockBackend extends MailApi {
  /** Messages "still on the server", per folder id, handed out by `loadOlder`. */
  readonly older: Map<string, Email[]>;
  emit<K extends keyof EventMap>(event: K, payload: EventMap[K]): void;
  /** Delivers a new message into an account's inbox and emits `mail:new`. */
  deliver(accountId: string, partial?: Partial<Email>): Email;
  readonly db: { accounts: Account[]; folders: Folder[]; emails: Email[]; tags: Tag[] };
}

const COLORS = ["#2563eb", "#7c3aed", "#db2777", "#ea580c", "#16a34a", "#0891b2", "#ca8a04"];
const SYSTEM: [FolderRole, string][] = [
  ["inbox", "Inbox"],
  ["sent", "Sent"],
  ["drafts", "Drafts"],
  ["archive", "Archive"],
  ["spam", "Spam"],
  ["trash", "Trash"],
];
const ROLE_ORDER: Record<FolderRole, number> = { inbox: 0, sent: 1, drafts: 2, archive: 3, spam: 4, trash: 5, custom: 6 };

/** Demo BIMI logos: a certified one and a plain one. */
const svgUrl = (svg: string) => `data:image/svg+xml;base64,${btoa(svg)}`;
const DEMO_LOGOS: Record<string, SenderLogo> = {
  "gmail.com": {
    svg: svgUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><path d="M10 18h44v30H10z" fill="#fff"/><path d="M10 18l22 17 22-17" fill="none" stroke="#ea4335" stroke-width="6"/></svg>'),
    verified: true,
    issuer: "DigiCert, Inc.",
  },
  "github.com": {
    svg: svgUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#24292f"/><circle cx="32" cy="30" r="14" fill="#fff"/><rect x="26" y="40" width="12" height="12" fill="#fff"/></svg>'),
    verified: true,
    issuer: "DigiCert, Inc.",
  },
  "stripe.com": {
    svg: svgUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#635bff"/><text x="32" y="44" font-size="34" font-family="Arial" font-weight="700" fill="#fff" text-anchor="middle">S</text></svg>'),
    verified: false,
    issuer: null,
  },
};

const SENDERS: Address[] = [
  { name: "Linear", address: "notifications@linear.app" },
  { name: "Anna Petrova", address: "anna@studio.design" },
  { name: "GitHub", address: "noreply@github.com" },
  { name: "Marcus Chen", address: "marcus@acme.io" },
  { name: "Figma", address: "team@figma.com" },
  { name: "Stripe", address: "receipts@stripe.com" },
  { name: "Sofia Rossi", address: "sofia.rossi@example.it" },
  { name: "Vercel", address: "ship@vercel.com" },
  { name: "Daniel Kim", address: "dkim@northwind.dev" },
  { name: "Notion", address: "digest@notion.so" },
];
const SUBJECTS = [
  "Weekly product sync — agenda",
  "Your invoice for September",
  "[trajectory/mail] PR #142: Add delta sync for IMAP",
  "Design review: new composer",
  "Re: Offsite logistics",
  "Q3 roadmap draft",
  "Deployment succeeded: production",
  "Coffee next week?",
  "Your weekly digest",
  "Contract — please review by Friday",
  "Security alert: new sign-in",
  "Photos from the weekend",
];
const BODIES = [
  "Hi team,\n\nHere is the agenda for this week's sync. We'll cover the release checklist, open bugs and the onboarding flow.\n\nBest,\nMarcus",
  "Thanks for your business! Your receipt is attached. Amount due: $49.00.",
  "The pipeline finished in 2m 31s. All checks passed and the preview is live.",
  "Hey! Are you free for a coffee on Tuesday? There's a new place near the office I've been wanting to try.",
  "Please find the updated contract attached. Let me know if you have questions about section 4.2.",
];

let seq = 0;
const uid = (p: string) => `${p}-${Date.now().toString(36)}-${(seq++).toString(36)}`;
const now = () => Math.floor(Date.now() / 1000);
const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
const textToHtml = (t: string) =>
  `<div style="font-family:system-ui;line-height:1.6">${t
    .split("\n\n")
    .map((p) => `<p>${escapeHtml(p).replace(/\n/g, "<br>")}</p>`)
    .join("")}</div>`;
const snippetOf = (t: string) => t.replace(/\s+/g, " ").trim().slice(0, 180);

function parseAddress(s: string): Address {
  const m = s.match(/^\s*"?([^"<]*?)"?\s*<([^>]+)>\s*$/);
  return m ? { name: m[1].trim() || null, address: m[2].trim() } : { name: null, address: s.trim() };
}

function detect(email: string): DetectedSettings {
  const domain = email.split("@")[1]?.toLowerCase() ?? "";
  if (!domain.includes(".")) throw { kind: "invalid", message: `'${email}' is not an email address` };
  const tls = (host: string, port: number): ServerConfig => ({ host, port, security: "tls" });
  if (["gmail.com", "googlemail.com"].includes(domain))
    return { provider: "Google", suggestedKind: "gmail", imap: tls("imap.gmail.com", 993), pop3: tls("pop.gmail.com", 995), smtp: tls("smtp.gmail.com", 465) };
  if (["outlook.com", "hotmail.com", "live.com"].includes(domain))
    return {
      provider: "Microsoft",
      suggestedKind: "outlook",
      imap: tls("outlook.office365.com", 993),
      pop3: tls("outlook.office365.com", 995),
      smtp: { host: "smtp.office365.com", port: 587, security: "start_tls" },
    };
  return { provider: null, suggestedKind: null, imap: tls(`imap.${domain}`, 993), pop3: tls(`pop.${domain}`, 995), smtp: tls(`smtp.${domain}`, 465) };
}

export function createMockBackend(opts: MockOptions = {}): MockBackend {
  const latency = opts.latencyMs ?? 0;
  const seedCount = opts.seedCount ?? 36;
  const db = { accounts: [] as Account[], folders: [] as Folder[], emails: [] as Email[], tags: [] as Tag[] };
  const listeners = new Map<string, Set<(p: unknown) => void>>();

  const delay = <T>(value: T | (() => T)): Promise<T> =>
    new Promise((resolve, reject) => {
      const run = () => {
        try {
          resolve(typeof value === "function" ? (value as () => T)() : value);
        } catch (e) {
          reject(e);
        }
      };
      if (latency > 0) setTimeout(run, latency);
      else queueMicrotask(run);
    });
  const fail = (kind: string, message: string) => {
    throw { kind, message };
  };

  const emit = <K extends keyof EventMap>(event: K, payload: EventMap[K]) => {
    listeners.get(event)?.forEach((cb) => cb(payload));
  };
  const folderBy = (accountId: string, role: FolderRole) => db.folders.find((f) => f.accountId === accountId && f.role === role)!;
  const email = (id: string) => db.emails.find((e) => e.id === id) ?? fail("not_found", `email ${id}`)!;
  const summary = ({ id, accountId, folderId, from, subject, snippet, date, isRead, isFlagged, hasAttachments, tags, note, logoDomain }: Email): EmailSummary => ({
    id, accountId, folderId, from, subject, snippet, date, isRead, isFlagged, hasAttachments, tags: [...tags], hasNote: !!note, logoDomain,
  });
  const tag = (id: string) => db.tags.find((t) => t.id === id) ?? fail("not_found", `tag ${id}`)!;
  const tagName = (name: string, except?: string) => {
    const n = name.split(/\s+/).filter(Boolean).join(" ");
    if (!n) fail("invalid", "a tag needs a name");
    if (n.length > 40) fail("invalid", "a tag name can be at most 40 characters");
    if (db.tags.some((t) => t.id !== except && t.name.toLowerCase() === n.toLowerCase())) fail("invalid", `a tag named '${n}' already exists`);
    return n;
  };
  const tagColor = (c: string) => (/^#[0-9a-f]{6}$/i.test(c.trim()) ? c.trim().toLowerCase() : fail("invalid", `'${c}' is not a #rrggbb colour`)!);
  const SORTS: Record<string, (a: Email, b: Email) => number> = {
    newest: (a, b) => b.date - a.date,
    oldest: (a, b) => a.date - b.date,
    unread: (a, b) => Number(a.isRead) - Number(b.isRead) || b.date - a.date,
    flagged: (a, b) => Number(b.isFlagged) - Number(a.isFlagged) || b.date - a.date,
    sender: (a, b) => (a.from.name || a.from.address).toLowerCase().localeCompare((b.from.name || b.from.address).toLowerCase()) || b.date - a.date,
    subject: (a, b) => a.subject.toLowerCase().localeCompare(b.subject.toLowerCase()) || b.date - a.date,
  };

  function createAccount(kind: AccountKind, displayName: string, address: string, extra: Partial<Account> = {}): Account {
    const account: Account = {
      id: uid("acc"),
      kind,
      displayName,
      email: address,
      incoming: null,
      outgoing: null,
      username: null,
      pop3DownloadDays: null,
      color: COLORS[db.accounts.length % COLORS.length],
      createdAt: now(),
      ...extra,
    };
    db.accounts.push(account);
    for (const [role, name] of SYSTEM) {
      db.folders.push({ id: uid("fld"), accountId: account.id, name, path: role === "inbox" ? "INBOX" : name, role, unreadCount: 0, totalCount: 0 });
    }
    db.folders.push({ id: uid("fld"), accountId: account.id, name: "Receipts", path: "Receipts", role: "custom", unreadCount: 0, totalCount: 0 });
    db.folders.push({ id: uid("fld"), accountId: account.id, name: "Projects", path: "Projects", role: "custom", unreadCount: 0, totalCount: 0 });
    seed(account, seedCount);
    return account;
  }

  function makeEmail(account: Account, folder: Folder, i: number, partial: Partial<Email> = {}): Email {
    const from = SENDERS[i % SENDERS.length];
    const body = BODIES[i % BODIES.length];
    const hasAttachments = i % 5 === 1;
    const id = uid("eml");
    return {
      id,
      accountId: account.id,
      folderId: folder.id,
      from: folder.role === "sent" ? { name: account.displayName, address: account.email } : from,
      subject: SUBJECTS[i % SUBJECTS.length],
      snippet: snippetOf(body),
      date: now() - i * 3600 * 5 - 120,
      isRead: i % 3 !== 0,
      isFlagged: i % 7 === 2,
      hasAttachments,
      to: [{ name: account.displayName, address: account.email }],
      cc: i % 4 === 0 ? [{ name: "Team", address: "team@example.com" }] : [],
      replyTo: [],
      bodyHtml: i % 2 === 0 ? textToHtml(body) : null,
      bodyText: body,
      messageId: `<${id}@mock.local>`,
      inReplyTo: null,
      tags: [],
      hasNote: false,
      note: null,
      // As if the receiving server reported DMARC pass for these senders.
      logoDomain: folder.role === "sent" ? null : (Object.keys(DEMO_LOGOS).find((d) => from.address.endsWith(`@${d}`)) ?? null),
      attachments: hasAttachments
        ? [{ id: uid("att"), emailId: id, filename: i % 2 ? "invoice.pdf" : "screenshot.png", mimeType: i % 2 ? "application/pdf" : "image/png", size: 182_340 + i * 1000, contentId: null }]
        : [],
      ...partial,
    };
  }

  function seed(account: Account, count: number) {
    const inbox = folderBy(account.id, "inbox");
    const sent = folderBy(account.id, "sent");
    const receipts = db.folders.find((f) => f.accountId === account.id && f.name === "Receipts")!;
    for (let i = 0; i < count; i++) db.emails.push(makeEmail(account, inbox, i));
    for (let i = 0; i < Math.min(4, count); i++) db.emails.push(makeEmail(account, sent, i + 3, { isRead: true }));
    for (let i = 0; i < Math.min(3, count); i++) db.emails.push(makeEmail(account, receipts, 1 + i * 5, { subject: "Your receipt", isRead: true }));
  }

  /** Me ⇄ Support, each reply quoting the previous message like Gmail does. */
  function seedThread(account: Account) {
    const me = { name: account.displayName, address: account.email };
    const support = { name: "Support", address: "support@stripe.com" };
    const steps: [Address, string][] = [
      [me, "Hi, I was charged twice for the March invoice. Could you take a look?"],
      [support, "Thanks for reaching out! Could you send the last four digits of the card?"],
      [me, "Sure, it's 4242."],
      [support, "Found it: the duplicate charge is refunded and should show up within 5 days."],
    ];
    let prev: Email | null = null;
    steps.forEach(([from, text], i) => {
      const mine = from === me;
      const quote = prev
        ? `<div class="gmail_quote"><div class="gmail_attr">On ${new Date(prev.date * 1000).toUTCString()}, ${prev.from.name} wrote:<br></div><blockquote class="gmail_quote">${prev.bodyHtml}</blockquote></div>`
        : "";
      const mail: Email = makeEmail(account, folderBy(account.id, mine ? "sent" : "inbox"), i, {
        from,
        to: [mine ? support : me],
        cc: [],
        subject: i ? "Re: Double charge" : "Double charge",
        bodyHtml: `<div>${text}</div><br>${quote}`,
        bodyText: text,
        snippet: text,
        date: now() - (3 - i) * 3600 * 20 - 600,
        isRead: i < 3,
        isFlagged: false,
        hasAttachments: false,
        attachments: [],
        inReplyTo: prev?.messageId ?? null,
        logoDomain: mine ? null : "stripe.com",
      });
      db.emails.push(mail);
      prev = mail;
    });
  }

  function withCounts(f: Folder): Folder {
    const mails = db.emails.filter((e) => e.folderId === f.id);
    return { ...f, totalCount: mails.length, unreadCount: mails.filter((e) => !e.isRead).length };
  }

  function validate(a: NewAccount) {
    detect(a.email);
    if (a.kind === "gmail" || a.kind === "outlook") fail("invalid", "use the OAuth sign-in for Google and Microsoft accounts");
    if (!a.incoming?.host || !a.outgoing?.host) fail("invalid", "server host and port are required");
    if (!a.password) fail("invalid", "password is required");
    // Mirrors net::is_private_host: plaintext only towards local / private hosts.
    const isPrivate = (h: string) =>
      /^(localhost|127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.|::1$|\[::1\]$)/i.test(h) ||
      /\.(local|lan|localhost|internal|home\.arpa)$/i.test(h) ||
      !h.includes(".");
    for (const c of [a.incoming, a.outgoing]) {
      if (c?.security === "none" && !isPrivate(c.host.trim()))
        fail("invalid", `${c.host.trim()} is a public server: an unencrypted connection would expose the password. Use SSL/TLS or STARTTLS.`);
    }
  }

  function storeOutgoing(out: OutgoingEmail, role: FolderRole): Email {
    const account = db.accounts.find((a) => a.id === out.accountId) ?? fail("not_found", "account")!;
    const folder = folderBy(account.id, role);
    if (out.draftId) db.emails = db.emails.filter((e) => e.id !== out.draftId);
    const id = uid("eml");
    const mail: Email = {
      id,
      accountId: account.id,
      folderId: folder.id,
      from: { name: account.displayName, address: account.email },
      subject: out.subject,
      snippet: snippetOf(out.bodyText),
      date: now(),
      isRead: true,
      isFlagged: false,
      hasAttachments: out.attachments.length > 0,
      to: out.to.filter(Boolean).map(parseAddress),
      cc: out.cc.filter(Boolean).map(parseAddress),
      replyTo: [],
      bodyHtml: out.bodyHtml,
      bodyText: out.bodyText,
      messageId: role === "sent" ? `<${id}@mock.local>` : null,
      inReplyTo: out.inReplyTo,
      tags: [],
      hasNote: false,
      note: null,
      logoDomain: null,
      attachments: out.attachments.map((a) => ({
        id: uid("att"),
        emailId: id,
        filename: a.filename,
        mimeType: a.mimeType,
        size: Math.floor((a.dataBase64.length * 3) / 4),
        contentId: null,
      })),
    };
    db.emails.push(mail);
    emit("mail:changed", { accountId: account.id });
    return mail;
  }

  if (opts.withDemoAccount) {
    createAccount("imap", "Alex Morgan", "alex@trajectory.dev", {
      incoming: { host: "imap.trajectory.dev", port: 993, security: "tls" },
      outgoing: { host: "smtp.trajectory.dev", port: 465, security: "tls" },
    });
    const inbox = db.emails.filter((e) => e.folderId === folderBy(db.accounts[0].id, "inbox").id);
    db.tags.push(
      { id: uid("tag"), name: "Follow up", color: "#ea580c", position: 0, count: 0 },
      { id: uid("tag"), name: "Work", color: "#2563eb", position: 1, count: 0 },
    );
    inbox[0].tags.push(db.tags[0].id, db.tags[1].id);
    inbox[2].tags.push(db.tags[1].id);
    inbox[0].note = "Answer before Friday's sync.";
    if (opts.demoThread) seedThread(db.accounts[0]);
    // A second mailbox, so the account switcher has something to switch to.
    if (opts.secondDemoAccount) createAccount("gmail", "Alex Morgan", "alex.morgan@gmail.com");
  }

  /** Sign-in apps "saved in the keychain" (the secret kept apart, as in Rust). */
  const oauthApps = new Map<OAuthProviderId, { clientId: string; secret: string | null }>();
  const oauthInfo = (provider: OAuthProviderId): OAuthAppInfo => {
    const app = oauthApps.get(provider);
    if (!app) return { provider, clientId: null, secretHint: null, source: null };
    const dash = app.secret?.indexOf("-") ?? -1;
    const hint = app.secret ? (dash >= 0 && dash <= 8 ? `${app.secret.slice(0, dash + 1)}••••••` : "••••••") : null;
    return { provider, clientId: app.clientId, secretHint: hint, source: "settings" };
  };

  const older = new Map<string, Email[]>();

  const backend: MockBackend = {
    db,
    older,
    emit,
    deliver(accountId, partial = {}) {
      const account = db.accounts.find((a) => a.id === accountId)!;
      const mail = makeEmail(account, folderBy(accountId, "inbox"), seq, { date: now(), isRead: false, ...partial });
      db.emails.push(mail);
      emit("mail:new", { accountId, count: 1, latest: [summary(mail)] });
      emit("mail:changed", { accountId });
      return mail;
    },

    listAccounts: () => delay(() => [...db.accounts]),
    addAccount: (a, verify = true) =>
      delay(() => {
        validate(a);
        if (db.accounts.some((x) => x.email.toLowerCase() === a.email.toLowerCase())) fail("invalid", `${a.email} is already added`);
        if (verify && a.password === "wrong") fail("auth", "IMAP login rejected: invalid credentials");
        return createAccount(a.kind, a.displayName || a.email.split("@")[0], a.email, {
          incoming: a.incoming,
          outgoing: a.outgoing,
          username: a.username,
          pop3DownloadDays: a.kind === "pop3" ? a.pop3DownloadDays : null,
        });
      }),
    addOAuthAccount: (kind) =>
      delay(() => {
        const address = kind === "gmail" ? "demo@gmail.com" : "demo@outlook.com";
        return db.accounts.find((a) => a.email === address) ?? createAccount(kind, "Demo", address);
      }),
    oauthApps: () => delay(() => (["google", "microsoft"] as const).map(oauthInfo)),
    setOAuthApp: (provider, clientId, clientSecret) =>
      delay(() => {
        const id = clientId.trim();
        const valid =
          provider === "google" ? /^[A-Za-z0-9-]+\.apps\.googleusercontent\.com$/.test(id) : /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(id);
        if (!valid)
          fail(
            "invalid",
            provider === "google"
              ? "a Google client ID ends with .apps.googleusercontent.com"
              : "a Microsoft client ID looks like 00000000-0000-0000-0000-000000000000",
          );
        const secret = clientSecret?.trim() || oauthApps.get(provider)?.secret || null;
        if (provider === "google" && !secret) fail("invalid", "a Google desktop client also needs its client secret");
        oauthApps.set(provider, { clientId: id, secret: provider === "google" ? secret : null });
        return oauthInfo(provider);
      }),
    clearOAuthApp: (provider) =>
      delay(() => {
        oauthApps.delete(provider);
        return oauthInfo(provider);
      }),
    removeAccount: (id) =>
      delay(() => {
        db.accounts = db.accounts.filter((a) => a.id !== id);
        db.folders = db.folders.filter((f) => f.accountId !== id);
        db.emails = db.emails.filter((e) => e.accountId !== id);
      }),
    testConnection: (a) =>
      delay(() => {
        validate(a);
        const ok = a.password !== "wrong";
        return {
          incomingOk: ok,
          outgoingOk: ok,
          incomingError: ok ? null : "IMAP login rejected: invalid credentials",
          outgoingError: ok ? null : "535 5.7.8 Authentication failed",
        };
      }),
    autodetect: (address) => delay(() => detect(address)),
    listFolders: (accountId) =>
      delay(() =>
        db.folders
          .filter((f) => f.accountId === accountId)
          .map(withCounts)
          .sort((a, b) => ROLE_ORDER[a.role] - ROLE_ORDER[b.role] || a.name.localeCompare(b.name)),
      ),
    listEmails: (q) =>
      delay(() => {
        const term = q.search?.trim().toLowerCase();
        const all = db.emails
          .filter((e) =>
            q.tagId
              ? e.tags.includes(q.tagId) && (!q.accountId || e.accountId === q.accountId)
              : q.folderId
                ? e.folderId === q.folderId
                : q.accountId
                  ? e.accountId === q.accountId
                  : true,
          )
          .filter((e) => (q.filter === "unread" ? !e.isRead : q.filter === "flagged" ? e.isFlagged : true))
          .filter(
            (e) =>
              !term ||
              [e.subject, e.from.address, e.from.name ?? "", e.snippet, e.bodyText ?? "", e.note ?? ""].some((s) => s.toLowerCase().includes(term)),
          )
          .sort((a, b) => SORTS[q.sort ?? "newest"](a, b) || a.id.localeCompare(b.id));
        const offset = q.offset ?? 0;
        const items = all.slice(offset, offset + (q.limit ?? 50)).map(summary);
        return { items, total: all.length, hasMore: offset + items.length < all.length };
      }),
    loadOlder: (folderId) =>
      delay(() => {
        const pool = older.get(folderId) ?? [];
        const batch = pool.splice(0, 50);
        db.emails.push(...batch);
        if (batch.length) emit("mail:changed", { accountId: batch[0].accountId });
        return { added: batch.length, hasMore: pool.length > 0 };
      }),
    getThread: (id) =>
      delay(() => {
        const start = email(id);
        const hidden = new Set(db.folders.filter((f) => ["trash", "spam", "drafts"].includes(f.role)).map((f) => f.id));
        const key = (m: string | null) => (m ?? "").replace(/[<>\s]/g, "").toLowerCase();
        const keys = new Set([key(start.messageId), key(start.inReplyTo)].filter(Boolean));
        const members = new Set([start.id]);
        for (let grew = true; grew && keys.size; ) {
          grew = false;
          for (const e of db.emails) {
            if (e.accountId !== start.accountId || hidden.has(e.folderId) || members.has(e.id)) continue;
            if (keys.has(key(e.messageId)) || keys.has(key(e.inReplyTo))) {
              members.add(e.id);
              for (const k of [key(e.messageId), key(e.inReplyTo)]) if (k) keys.add(k);
              grew = true;
            }
          }
        }
        return db.emails.filter((e) => members.has(e.id)).sort((a, b) => a.date - b.date).map(summary);
      }),
    getEmail: (id) => delay(() => ({ ...email(id), tags: [...email(id).tags], hasNote: !!email(id).note, attachments: [...email(id).attachments] })),
    senderLogo: (domain) => delay(() => DEMO_LOGOS[domain.toLowerCase()] ?? { svg: null, verified: false, issuer: null }),
    listTags: (accountId) =>
      delay(() =>
        [...db.tags]
          .sort((a, b) => a.position - b.position)
          .map((t) => ({ ...t, count: db.emails.filter((e) => e.tags.includes(t.id) && (!accountId || e.accountId === accountId)).length })),
      ),
    createTag: (name, color) =>
      delay(() => {
        const t: Tag = { id: uid("tag"), name: tagName(name), color: tagColor(color), position: db.tags.length ? Math.max(...db.tags.map((x) => x.position)) + 1 : 0, count: 0 };
        db.tags.push(t);
        return { ...t };
      }),
    updateTag: (id, name, color) =>
      delay(() => {
        const t = tag(id);
        Object.assign(t, { name: tagName(name, id), color: tagColor(color) });
        return { ...t, count: db.emails.filter((e) => e.tags.includes(id)).length };
      }),
    deleteTag: (id) =>
      delay(() => {
        tag(id);
        db.tags = db.tags.filter((t) => t.id !== id);
        for (const e of db.emails) e.tags = e.tags.filter((t) => t !== id);
      }),
    reorderTags: (ids) =>
      delay(() => {
        const rest = [...db.tags].sort((a, b) => a.position - b.position).filter((t) => !ids.includes(t.id));
        [...ids.map(tag), ...rest].forEach((t, i) => (t.position = i));
      }),
    setEmailTag: (emailId, tagId, on) =>
      delay(() => {
        const e = email(emailId);
        tag(tagId);
        e.tags = on ? [...new Set([...e.tags, tagId])] : e.tags.filter((t) => t !== tagId);
      }),
    setNote: (emailId, note) =>
      delay(() => {
        const n = note?.trim() || null;
        if (n && n.length > 10_000) fail("invalid", "a note can be at most 10000 characters");
        email(emailId).note = n;
      }),
    markRead: (id, read) => delay(() => void (email(id).isRead = read)),
    setFlagged: (id, flagged) => delay(() => void (email(id).isFlagged = flagged)),
    moveEmail: (id, folderId) => delay(() => void (email(id).folderId = folderId)),
    archiveEmail: (id) => delay(() => void (email(id).folderId = folderBy(email(id).accountId, "archive").id)),
    deleteEmail: (id) =>
      delay(() => {
        const e = email(id);
        const role = db.folders.find((f) => f.id === e.folderId)?.role;
        if (role === "trash" || role === "drafts") db.emails = db.emails.filter((x) => x.id !== id);
        else e.folderId = folderBy(e.accountId, "trash").id;
      }),
    sendEmail: (out) =>
      delay(() => {
        if (!out.to.some((t) => t.trim())) fail("invalid", "add at least one recipient");
        return storeOutgoing(out, "sent").id;
      }),
    saveDraft: (out) => delay(() => storeOutgoing(out, "drafts").id),
    syncNow: (accountId) =>
      delay(() => {
        const ids = accountId ? [accountId] : db.accounts.map((a) => a.id);
        for (const id of ids) {
          emit("sync:status", { accountId: id, state: "syncing" });
          setTimeout(() => emit("sync:status", { accountId: id, state: "idle", newMessages: 0 }), latency * 4);
        }
      }),
    saveAttachment: (id) =>
      delay(() => {
        const att = db.emails.flatMap((e) => e.attachments).find((a) => a.id === id) ?? fail("not_found", "attachment")!;
        return `~/Downloads/${att.filename}`;
      }),
    openExternal: (url) => delay(() => void window.open(url, "_blank", "noopener")),
    on: async (event, cb) => {
      const set = listeners.get(event) ?? new Set();
      set.add(cb as (p: unknown) => void);
      listeners.set(event, set);
      const unlisten: Unlisten = () => set.delete(cb as (p: unknown) => void);
      return unlisten;
    },
  };
  return backend;
}
