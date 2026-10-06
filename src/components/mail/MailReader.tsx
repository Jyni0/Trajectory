import { useEffect, useMemo, useRef, useState } from "react";
import { motion } from "motion/react";
import {
  Archive,
  StickyNote,
  Tag as TagIcon,
  Download,
  File,
  FileImage,
  FileText,
  Flag,
  Forward,
  ImageOff,
  Mail,
  MailOpen,
  Reply,
  ReplyAll,
  Trash2,
} from "lucide-react";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { api, errorMessage } from "@/lib/api";
import { forwardDraft, replyDraft } from "@/lib/compose";
import { displayName, formatAddress, formatFullDate, formatListDate, formatSize, hasRemoteContent } from "@/lib/format";
import { splitHtmlQuote, splitTextQuote } from "@/lib/quotes";
import type { AttachmentMeta, Email, EmailSummary } from "@/lib/types";
import { TAG_PICKER_EVENT } from "@/hooks/useHotkeys";
import { Alert, Button, IconButton, Kbd, ScrollArea, Spinner, TextArea, cx } from "@/components";
import { HtmlViewer } from "./HtmlViewer";
import { TagChip, TagPicker, useTagsOf } from "./Tags";
import { SenderAvatar, VerifiedMark } from "./SenderAvatar";

/**
 * The private note on a message: saved as you type (debounced) and on blur;
 * clearing the text removes it. Stays on this device.
 */
function NoteCard({ email, autoFocus, onDone }: { email: Email; autoFocus: boolean; onDone: () => void }) {
  const setNote = useMailStore((s) => s.setNote);
  const pushToast = useUiStore((s) => s.pushToast);
  const [text, setText] = useState(email.note ?? "");
  const saved = useRef(email.note ?? "");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const latest = useRef(text);
  latest.current = text;

  const flush = () => {
    clearTimeout(timer.current);
    const value = latest.current.trim();
    if (value === saved.current.trim()) return;
    saved.current = value;
    void setNote(email.id, value).catch((e) => pushToast(errorMessage(e), "error"));
  };
  // Save what was typed when the message changes or the reader closes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => flush, []);

  return (
    <section aria-label="Note" className="flex flex-col gap-1.5 rounded-2xl border border-[var(--border-soft)] bg-[var(--bg-surface)] p-3">
      <div className="flex items-center gap-2 text-[12px] text-[var(--text-dim)]">
        <StickyNote size={13} />
        <span className="font-medium text-[var(--text-muted)]">Note</span>
        <span>· only on this device</span>
      </div>
      <TextArea
        aria-label="Note"
        autoFocus={autoFocus}
        rows={Math.min(8, Math.max(2, text.split("\n").length))}
        maxLength={10_000}
        placeholder="Add a private note…"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          clearTimeout(timer.current);
          timer.current = setTimeout(flush, 700);
        }}
        onBlur={() => {
          flush();
          if (!latest.current.trim()) onDone();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            (e.target as HTMLTextAreaElement).blur();
          }
        }}
        className="selectable resize-none border-0 bg-transparent px-0 text-[13px] leading-relaxed shadow-none focus:ring-0"
      />
    </section>
  );
}

function AttachmentIcon({ mime }: { mime: string }) {
  if (mime.startsWith("image/")) return <FileImage size={16} />;
  if (mime === "application/pdf" || mime.startsWith("text/")) return <FileText size={16} />;
  return <File size={16} />;
}

export function AttachmentList({ attachments }: { attachments: AttachmentMeta[] }) {
  const pushToast = useUiStore((s) => s.pushToast);
  const [busy, setBusy] = useState<string | null>(null);
  if (attachments.length === 0) return null;

  const save = async (a: AttachmentMeta) => {
    setBusy(a.id);
    try {
      const path = await api.saveAttachment(a.id);
      pushToast(`Saved to ${path}`, "success");
    } catch (e) {
      pushToast(errorMessage(e), "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-label="Attachments" className="flex flex-col gap-2">
      <h3 className="text-[11px] font-medium uppercase tracking-wide text-[var(--text-dim)]">
        {attachments.length} attachment{attachments.length > 1 && "s"}
      </h3>
      <div className="flex flex-wrap gap-2">
        {attachments.map((a) => (
          <button
            key={a.id}
            type="button"
            onClick={() => void save(a)}
            className="group flex h-12 max-w-[260px] items-center gap-3 rounded-xl bg-[var(--bg-input)] px-3 text-left transition-colors hover:bg-[var(--bg-elevated)]"
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-[var(--accent)]/15 text-[var(--accent)]">
              <AttachmentIcon mime={a.mimeType} />
            </span>
            <span className="flex min-w-0 flex-col leading-tight">
              <span className="truncate text-[13px] text-[var(--text-main)]">{a.filename}</span>
              <span className="text-[11px] text-[var(--text-dim)]">{formatSize(a.size)}</span>
            </span>
            {busy === a.id ? <Spinner size={14} /> : <Download size={14} className="shrink-0 text-[var(--text-dim)] group-hover:text-[var(--text-main)]" />}
          </button>
        ))}
      </div>
    </section>
  );
}

/**
 * Body and attachments of one message. In a conversation (`foldQuotes`) the
 * earlier messages it quotes are already shown above, so the quote is folded
 * behind a "•••" button, as Gmail does.
 */
function MessageContent({ email, foldQuotes }: { email: Email; foldQuotes: boolean }) {
  const loadRemoteDefault = useUiStore((s) => s.prefs.loadRemoteImages);
  const [remote, setRemote] = useState(false);
  const [showQuote, setShowQuote] = useState(false);
  const split = useMemo(
    () => (!foldQuotes ? null : email.bodyHtml ? splitHtmlQuote(email.bodyHtml) : splitTextQuote(email.bodyText ?? "")),
    [foldQuotes, email.bodyHtml, email.bodyText],
  );
  const folded = !!split?.quoted && !showQuote;
  const allowRemote = loadRemoteDefault || remote;
  const html = email.bodyHtml ? (folded ? split!.body : email.bodyHtml) : null;
  const text = folded ? split!.body : email.bodyText;

  return (
    <>
      {!allowRemote && !!html && hasRemoteContent(html) && (
        <Alert tone="info" icon={<ImageOff size={14} />}>
          <div className="flex items-center gap-3">
            <span className="flex-1">Remote images are blocked to protect your privacy.</span>
            <Button size="xs" onClick={() => setRemote(true)}>
              Load images
            </Button>
          </div>
        </Alert>
      )}

      <div className="flex flex-col gap-2">
        {html ? (
          <HtmlViewer html={html} allowRemote={allowRemote} />
        ) : (
          <div data-testid="email-body" className="selectable whitespace-pre-wrap break-words text-[14px] leading-relaxed text-[var(--text-main)]">
            {text}
          </div>
        )}
        {split?.quoted && (
          <button
            type="button"
            data-testid="toggle-quote"
            aria-label={showQuote ? "Hide quoted text" : "Show quoted text"}
            title={showQuote ? "Hide quoted text" : "Show quoted text"}
            aria-expanded={showQuote}
            onClick={() => setShowQuote((v) => !v)}
            className="flex h-4 w-7 items-center justify-center self-start rounded-md bg-[var(--bg-input)] text-[11px] leading-none tracking-widest text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-elevated)] hover:text-[var(--text-main)]"
          >
            •••
          </button>
        )}
      </div>

      <AttachmentList attachments={email.attachments} />
    </>
  );
}

/** "to me, Anna · cc Team" — the recipients line under the sender, Gmail style. */
function recipientsLine(email: Email, me: string): string {
  const name = (a: Email["to"][number]) => (a.address.toLowerCase() === me.toLowerCase() ? "me" : displayName(a));
  const to = email.to.map(name).join(", ");
  const cc = email.cc.map(name).join(", ");
  return [to && `to ${to}`, cc && `cc ${cc}`].filter(Boolean).join(" · ");
}

/** Room left of the body so it lines up with the sender name (padding + avatar + gap). */
const BODY_INDENT = "pl-[60px]";

/**
 * One message of a conversation, Gmail style: when collapsed a single line
 * (sender, start of the text, date); open, the sender with address and
 * recipients, and below, lined up with the name, the body. A single message
 * is shown the same way, as a conversation of one.
 */
function ThreadMessage({
  summary,
  email,
  me,
  initiallyOpen,
  foldQuotes,
  current,
  extras,
  articleRef,
}: {
  summary: EmailSummary;
  /** The full message when already loaded (the opened one). */
  email?: Email;
  me: string;
  initiallyOpen: boolean;
  foldQuotes: boolean;
  current?: boolean;
  /** Shown above the body when open (tags, note). */
  extras?: React.ReactNode;
  articleRef?: React.Ref<HTMLElement>;
}) {
  const pushToast = useUiStore((s) => s.pushToast);
  const [open, setOpen] = useState(initiallyOpen);
  const [loaded, setLoaded] = useState<Email | null>(null);
  const full = email ?? loaded;

  useEffect(() => {
    if (!open || full) return;
    let alive = true;
    api.getEmail(summary.id).then(
      (e) => alive && setLoaded(e),
      (e) => {
        if (!alive) return;
        pushToast(errorMessage(e), "error");
        setOpen(false);
      },
    );
    return () => {
      alive = false;
    };
  }, [open, full, summary.id, pushToast]);

  const name = displayName(summary.from);
  return (
    <article ref={articleRef} data-testid="thread-message" aria-current={current || undefined} className="scroll-mt-4 py-1">
      <button
        type="button"
        aria-expanded={open}
        aria-label={`${open ? "Collapse" : "Expand"} message from ${name}`}
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors hover:bg-[var(--hover-bg)]"
      >
        <SenderAvatar from={summary.from} logoDomain={summary.logoDomain} size={36} />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5 leading-tight">
          <span className="flex min-w-0 items-baseline gap-2">
            <span className="flex min-w-0 max-w-[60%] shrink-0 items-center gap-1 self-center">
              <VerifiedMark logoDomain={summary.logoDomain} size={14} />
              <span className={cx("truncate text-[13.5px] text-[var(--text-main)]", (open || !summary.isRead) && "font-semibold")}>{name}</span>
            </span>
            {open && <span className="min-w-0 truncate text-[12px] text-[var(--text-dim)]">&lt;{summary.from.address}&gt;</span>}
            <time
              className="ml-auto shrink-0 pl-2 text-[12px] text-[var(--text-dim)]"
              dateTime={new Date(summary.date * 1000).toISOString()}
              title={formatFullDate(summary.date)}
            >
              {open ? formatFullDate(summary.date) : formatListDate(summary.date)}
            </time>
          </span>
          <span className="truncate text-[12.5px] text-[var(--text-dim)]" title={open && full ? [...full.to, ...full.cc].map(formatAddress).join(", ") : undefined}>
            {open ? (full ? recipientsLine(full, me) : "") : summary.snippet || "(no text)"}
          </span>
        </span>
      </button>
      {open && (
        <div className={cx("flex flex-col gap-4 pb-4 pr-3 pt-2", BODY_INDENT)}>
          {extras}
          {full ? <MessageContent key={full.id} email={full} foldQuotes={foldQuotes} /> : <Spinner size={16} className="my-2 text-[var(--text-dim)]" />}
        </div>
      )}
    </article>
  );
}

/** The conversation of `id`, oldest first; null until loaded. */
function useThread(id: string | undefined): EmailSummary[] | null {
  const [thread, setThread] = useState<{ id: string; items: EmailSummary[] } | null>(null);
  // Reloaded when the mailbox changes (a reply sent, new mail synced).
  const version = useMailStore((s) => s.emails);
  useEffect(() => {
    if (!id) return;
    let alive = true;
    api.getThread(id).then(
      (items) => alive && setThread({ id, items }),
      () => alive && setThread({ id, items: [] }),
    );
    return () => {
      alive = false;
    };
  }, [id, version]);
  return thread && thread.id === id ? thread.items : null;
}

export function MailReader() {
  const email = useMailStore((s) => s.activeEmail);
  const loading = useMailStore((s) => s.loadingEmail);
  const selectedId = useMailStore((s) => s.selectedEmailId);
  const account = useMailStore((s) => s.accounts.find((a) => a.id === s.activeAccountId));
  const { archive, remove, toggleFlag, markRead } = useMailStore.getState();
  const openComposer = useUiStore((s) => s.openComposer);
  const tagEmail = useMailStore((s) => s.tagEmail);
  const tags = useTagsOf(email?.tags ?? NO_IDS);
  const [pickerOpen, setPickerOpen] = useState(false);
  const tagButton = useRef<HTMLButtonElement>(null);
  /** Note editor shown for a message that has no note yet. */
  const [noteFor, setNoteFor] = useState<string | null>(null);
  const loadedThread = useThread(email?.id);
  const activeRef = useRef<HTMLElement>(null);
  const thread = email && loadedThread?.some((m) => m.id === email.id) ? loadedThread : email ? [email] : [];
  const threaded = thread.length > 1;

  useEffect(() => {
    const open = () => setPickerOpen(true);
    window.addEventListener(TAG_PICKER_EVENT, open);
    return () => window.removeEventListener(TAG_PICKER_EVENT, open);
  }, []);
  useEffect(() => setPickerOpen(false), [selectedId]);
  // In a long conversation, start at the opened message, not the oldest.
  useEffect(() => {
    if (threaded && thread[0].id !== email?.id) activeRef.current?.scrollIntoView?.({ block: "start" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threaded, email?.id]);

  if (!selectedId) {
    return (
      <section className="flex min-w-0 flex-1 flex-col items-center justify-center gap-1.5 text-center" aria-label="Reader">
        <Mail size={18} strokeWidth={1.6} className="mb-1.5 text-[var(--text-dim)]" />
        <p className="text-[13px] font-medium text-[var(--text-main)]">Select a message to read</p>
        <p className="flex items-center gap-1.5 text-[12px] text-[var(--text-dim)]">
          <Kbd>J</Kbd> <Kbd>K</Kbd> to navigate · <Kbd>C</Kbd> to compose
        </p>
      </section>
    );
  }
  if (!email || email.id !== selectedId) {
    return (
      <section className="flex min-w-0 flex-1 items-center justify-center" aria-label="Reader">
        {loading && <Spinner size={18} className="text-[var(--text-dim)]" />}
      </section>
    );
  }

  const accountEmail = account?.email ?? "";
  const extras = (tags.length > 0 || email.note || noteFor === email.id) && (
    <>
      {tags.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5" aria-label="Tags">
          {tags.map((t) => (
            <TagChip key={t.id} tag={t} onRemove={() => void tagEmail(email.id, t.id, false)} />
          ))}
        </div>
      )}
      {(email.note || noteFor === email.id) && (
        <NoteCard key={email.id} email={email} autoFocus={noteFor === email.id && !email.note} onDone={() => setNoteFor(null)} />
      )}
    </>
  );

  return (
    <section className="flex min-w-0 flex-1 flex-col overflow-hidden" aria-label="Reader">
      <div className="flex h-12 shrink-0 items-center gap-0.5 border-b border-[var(--border-soft)] px-3" role="toolbar" aria-label="Message actions">
        <IconButton label="Reply" onClick={() => openComposer(replyDraft(email, accountEmail))}>
          <Reply size={15} />
        </IconButton>
        <IconButton label="Reply all" onClick={() => openComposer(replyDraft(email, accountEmail, true))}>
          <ReplyAll size={15} />
        </IconButton>
        <IconButton label="Forward" onClick={() => openComposer(forwardDraft(email))}>
          <Forward size={15} />
        </IconButton>
        <div className="mx-1.5 h-4 w-px bg-[var(--border)]" />
        <IconButton label="Archive" onClick={() => void archive(email.id)}>
          <Archive size={15} />
        </IconButton>
        <IconButton label="Delete" onClick={() => void remove(email.id)}>
          <Trash2 size={15} />
        </IconButton>
        <div className="flex-1" />
        <IconButton ref={tagButton} label="Tags (T)" onClick={() => setPickerOpen((o) => !o)} className={cx(pickerOpen && "bg-[var(--hover-bg)]")}>
          <TagIcon size={15} />
        </IconButton>
        <IconButton label={email.note ? "Edit note" : "Add note"} onClick={() => setNoteFor(email.id)} className={cx(!!email.note && "text-[var(--accent)]")}>
          <StickyNote size={15} />
        </IconButton>
        <IconButton label={email.isRead ? "Mark as unread" : "Mark as read"} onClick={() => void markRead(email.id, !email.isRead)}>
          {email.isRead ? <Mail size={15} /> : <MailOpen size={15} />}
        </IconButton>
        <IconButton
          label={email.isFlagged ? "Unflag" : "Flag"}
          onClick={() => void toggleFlag(email.id)}
          className={cx(email.isFlagged && "text-amber-500 hover:text-amber-500")}
        >
          <Flag size={15} fill={email.isFlagged ? "currentColor" : "none"} />
        </IconButton>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <motion.div
          key={email.id}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.18 }}
          className="mx-auto flex max-w-3xl flex-col gap-3 px-5 py-5"
        >
          <div className={cx("flex items-baseline gap-3 pr-3", BODY_INDENT)}>
            <h2 className="flex-1 text-[20px] font-semibold leading-snug text-[var(--text-main)]" data-testid="reader-subject">
              {email.subject || "(no subject)"}
            </h2>
            {threaded && (
              <span className="shrink-0 text-[12px] text-[var(--text-dim)]" data-testid="thread-count">
                {thread.length} messages
              </span>
            )}
          </div>

          <div className="flex flex-col divide-y divide-[var(--border-soft)]" role="list" aria-label="Conversation">
            {thread.map((m) => {
              const isActive = m.id === email.id;
              return (
                <div role="listitem" key={m.id}>
                  <ThreadMessage
                    articleRef={isActive ? activeRef : undefined}
                    summary={isActive ? email : m}
                    email={isActive ? email : undefined}
                    me={accountEmail}
                    initiallyOpen={isActive}
                    foldQuotes={threaded}
                    current={threaded && isActive}
                    extras={isActive ? extras : undefined}
                  />
                </div>
              );
            })}
          </div>

          <div className={cx("flex gap-2 pt-2", BODY_INDENT)}>
            <Button icon={<Reply size={14} />} onClick={() => openComposer(replyDraft(email, accountEmail))}>
              Reply
            </Button>
            <Button icon={<Forward size={14} />} onClick={() => openComposer(forwardDraft(email))}>
              Forward
            </Button>
          </div>
        </motion.div>
      </ScrollArea>
      <TagPicker emailId={email.id} selected={email.tags} open={pickerOpen} anchor={tagButton} onClose={() => setPickerOpen(false)} />
    </section>
  );
}

const NO_IDS: string[] = [];
