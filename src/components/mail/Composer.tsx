import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { motion } from "motion/react";
import {
  Bold,
  FilePen,
  Italic,
  Link2,
  List,
  ListOrdered,
  Paperclip,
  PenSquare,
  Quote,
  Reply,
  Save,
  Send,
  Trash2,
  Underline,
  Users,
  X,
} from "lucide-react";
import { setComposerLeave, useUiStore } from "@/store/uiStore";
import { useMailStore } from "@/store/mailStore";
import { api, errorMessage } from "@/lib/api";
import { fileToAttachment, htmlToPlainText, isValidRecipient, splitRecipients, type ComposerDraft } from "@/lib/compose";
import { formatSize } from "@/lib/format";
import type { OutgoingEmail } from "@/lib/types";
import { Alert, Button, Combobox, IconButton, Input, Kbd, ScrollArea, Spinner, cx } from "@/components";

const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024;

/** Label column of the recipient rows. */
const LABEL = "w-14 shrink-0 text-[11px] font-medium text-[var(--text-muted)]";

/** A recipients field: typed addresses become chips (Enter , ; Tab or blur). */
export function RecipientInput({
  label,
  values,
  onChange,
  autoFocus,
  trailing,
}: {
  label: string;
  values: string[];
  onChange: (v: string[]) => void;
  autoFocus?: boolean;
  trailing?: React.ReactNode;
}) {
  const [text, setText] = useState("");
  const commit = () => {
    const parts = splitRecipients(text);
    if (parts.length) onChange([...values, ...parts.filter((p) => !values.includes(p))]);
    setText("");
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (["Enter", ",", ";", "Tab"].includes(e.key) && text.trim()) {
      if (e.key !== "Tab") e.preventDefault();
      commit();
    } else if (e.key === "Backspace" && !text && values.length) {
      onChange(values.slice(0, -1));
    }
  };
  return (
    <div className="flex items-center gap-2">
      <span className={LABEL}>{label}</span>
      <div className="flex min-h-9 min-w-0 flex-1 flex-wrap items-center gap-1 rounded-xl border border-transparent bg-[var(--bg-input)] px-1.5 py-1 transition-colors focus-within:border-[var(--accent)] hover:border-[var(--border)]">
        {values.map((v) => {
          const ok = isValidRecipient(v);
          return (
            <span
              key={v}
              data-testid="recipient-chip"
              data-invalid={!ok || undefined}
              className={cx(
                "inline-flex h-6 items-center gap-1 rounded-lg pl-2 pr-0.5 text-[12px]",
                ok ? "bg-[var(--bg-elevated)] text-[var(--text-main)]" : "bg-[var(--diff-del)]/15 text-[var(--diff-del)]",
              )}
            >
              {v}
              <button
                type="button"
                aria-label={`Remove ${v}`}
                onClick={() => onChange(values.filter((x) => x !== v))}
                className="rounded-md p-0.5 hover:bg-[var(--hover-bg)]"
              >
                <X size={11} />
              </button>
            </span>
          );
        })}
        <input
          aria-label={label}
          autoFocus={autoFocus}
          data-autofocus={autoFocus || undefined}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          onBlur={commit}
          className="h-7 min-w-[120px] flex-1 bg-transparent px-1.5 text-[12px] text-[var(--text-main)] outline-none placeholder:text-[var(--text-dim)]"
          placeholder={values.length ? "" : "name@example.com"}
        />
        {trailing}
      </div>
    </div>
  );
}

function ToolbarButton({ label, cmd, arg, children }: { label: string; cmd: string; arg?: string; children: React.ReactNode }) {
  return (
    <IconButton
      label={label}
      size="xs"
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => {
        const value = cmd === "createLink" ? window.prompt("Link URL", "https://") : arg;
        if (cmd === "createLink" && !value) return;
        document.execCommand(cmd, false, value ?? undefined);
      }}
    >
      {children}
    </IconButton>
  );
}

const same = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Anything written since the composer opened (a new, a reply or a reopened draft). */
function isChanged(d: ComposerDraft, html: string, from: ComposerDraft, fromHtml: string) {
  return (
    !same(d.to, from.to) ||
    !same(d.cc, from.cc) ||
    !same(d.bcc, from.bcc) ||
    d.subject.trim() !== from.subject.trim() ||
    d.accountId !== from.accountId ||
    d.attachments.length !== from.attachments.length ||
    htmlToPlainText(html) !== htmlToPlainText(fromHtml)
  );
}

/**
 * The composer opens in place of the reading pane, laid out like
 * Gravitation's edit panel: icon header, cards of fields, actions pinned at
 * the bottom. Key it by the draft (see composerKey).
 */
export const composerKey = (d: ComposerDraft) => d.draftId ?? d.subject + d.accountId;

export function Composer({ initial }: { initial: ComposerDraft }) {
  const closeComposer = useUiStore((s) => s.closeComposer);
  const pushToast = useUiStore((s) => s.pushToast);
  const accounts = useMailStore((s) => s.accounts);
  const [draft, setDraft] = useState<ComposerDraft>(initial);
  const [showCc, setShowCc] = useState(initial.cc.length > 0 || initial.bcc.length > 0);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const editorRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const asideRef = useRef<HTMLElement>(null);

  /** The body as the editor first showed it — "changed" compares against it. */
  const initialHtml = useRef(initial.html);
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.innerHTML = initial.html;
      initialHtml.current = editorRef.current.innerHTML;
    }
  }, [initial.html]);

  // First focus: the first empty part (To, then Subject, then the body).
  useEffect(() => {
    asideRef.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  }, []);

  const patch = (p: Partial<ComposerDraft>) => setDraft((d) => ({ ...d, ...p }));
  const currentHtml = () => editorRef.current?.innerHTML ?? draft.html;

  const toOutgoing = (): OutgoingEmail => {
    const html = currentHtml();
    return {
      accountId: draft.accountId,
      to: draft.to,
      cc: draft.cc,
      bcc: draft.bcc,
      subject: draft.subject,
      bodyText: htmlToPlainText(html),
      bodyHtml: html.trim() ? html : null,
      attachments: draft.attachments,
      inReplyTo: draft.inReplyTo,
      draftId: draft.draftId,
    };
  };

  const refreshAfterChange = async () => {
    const store = useMailStore.getState();
    await store.refreshFolders();
    await store.loadEmails();
  };

  const send = async () => {
    setError(null);
    if (draft.to.length === 0) return setError("Add at least one recipient.");
    const invalid = [...draft.to, ...draft.cc, ...draft.bcc].find((r) => !isValidRecipient(r));
    if (invalid) return setError(`"${invalid}" is not a valid address.`);
    setSending(true);
    try {
      await api.sendEmail(toOutgoing());
      closeComposer();
      pushToast("Message sent", "success");
      await refreshAfterChange();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSending(false);
    }
  };

  const saveDraft = async (silent = false) => {
    try {
      const id = await api.saveDraft(toOutgoing());
      patch({ draftId: id });
      if (!silent) pushToast("Draft saved", "success");
      await refreshAfterChange();
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    }
  };

  const close = async () => {
    if (isChanged(draft, currentHtml(), initial, initialHtml.current) && !sending) {
      if (!(await saveDraft(true))) return false; // keep it open: the error is shown
      pushToast("Draft saved", "info");
    }
    closeComposer();
    return true;
  };

  // Opening a message or another draft leaves the composer the same way.
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    setComposerLeave(() => closeRef.current());
    return () => setComposerLeave(null);
  }, []);

  const discard = async () => {
    if (draft.draftId) {
      await api.deleteEmail(draft.draftId).catch(() => undefined);
      await refreshAfterChange();
    }
    closeComposer();
  };

  const addFiles = async (files: FileList | null) => {
    if (!files) return;
    const list = Array.from(files);
    const tooBig = list.find((f) => f.size > MAX_ATTACHMENT_BYTES);
    if (tooBig) return setError(`${tooBig.name} is larger than 25 MB.`);
    const atts = await Promise.all(list.map(fileToAttachment));
    patch({ attachments: [...draft.attachments, ...atts] });
  };

  const TitleIcon = draft.inReplyTo ? Reply : draft.draftId ? FilePen : PenSquare;
  const title = draft.inReplyTo ? "Reply" : draft.draftId ? "Edit draft" : "New message";
  const from = accounts.find((a) => a.id === draft.accountId);

  return (
    <motion.section
      ref={asideRef}
      aria-label="Composer"
      className="selectable flex min-w-0 flex-1 flex-col overflow-hidden"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          void send();
        } else if (e.key === "Escape") {
          e.preventDefault();
          void close();
        }
      }}
    >
      {/* Header: what is being written, in words and an icon */}
      <div className="mx-auto flex w-full max-w-[860px] shrink-0 items-center gap-3 px-4 pb-3 pt-4">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent)]/15 text-[var(--accent)]">
          <TitleIcon size={18} strokeWidth={1.7} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-semibold text-[var(--text-main)]">{draft.subject.trim() || title}</div>
          <div className="truncate text-[11.5px] text-[var(--text-dim)]">{from ? `${title} · from ${from.email}` : title}</div>
        </div>
        <IconButton label="Close composer" onClick={() => void close()}>
          <X size={14} />
        </IconButton>
      </div>

      <ScrollArea className="min-h-0 flex-1" innerClassName="mx-auto flex min-h-full w-full max-w-[860px] flex-col gap-3 px-4 pb-4 pt-1">
        {/* Who and what */}
        <section className="flex flex-col gap-2.5 rounded-2xl border border-[var(--border-soft)] bg-[var(--bg-surface)] p-4">
          <div className="flex min-h-5 items-center gap-2">
            <Users size={14} strokeWidth={1.7} className="shrink-0 text-[var(--text-dim)]" />
            <span className="text-[12.5px] font-medium text-[var(--text-main)]">Recipients</span>
            {!showCc && (
              <button
                type="button"
                onClick={() => setShowCc(true)}
                className="ml-auto text-[11px] text-[var(--text-dim)] transition-colors hover:text-[var(--text-main)]"
              >
                Cc / Bcc
              </button>
            )}
          </div>
          {accounts.length > 1 && (
            <div className="flex items-center gap-2">
              <span className={LABEL}>From</span>
              <div className="min-w-0 flex-1">
                <Combobox
                  searchable={false}
                  value={draft.accountId}
                  onChange={(accountId) => patch({ accountId })}
                  options={accounts.map((a) => ({ value: a.id, label: `${a.displayName} <${a.email}>` }))}
                />
              </div>
            </div>
          )}
          <RecipientInput label="To" values={draft.to} onChange={(to) => patch({ to })} autoFocus={draft.to.length === 0} />
          {showCc && (
            <>
              <RecipientInput label="Cc" values={draft.cc} onChange={(cc) => patch({ cc })} />
              <RecipientInput label="Bcc" values={draft.bcc} onChange={(bcc) => patch({ bcc })} />
            </>
          )}
          <div className="flex items-center gap-2">
            <span className={LABEL}>Subject</span>
            <Input
              aria-label="Subject"
              value={draft.subject}
              onChange={(e) => patch({ subject: e.target.value })}
              data-autofocus={draft.to.length > 0 && !draft.subject ? true : undefined}
              placeholder="What's this about?"
            />
          </div>
        </section>

        {/* The message */}
        <section className="flex min-h-[260px] flex-1 flex-col rounded-2xl border border-[var(--border-soft)] bg-[var(--bg-surface)]">
          <div className="flex items-center gap-0.5 border-b border-[var(--border-soft)] px-2 py-1.5" role="toolbar" aria-label="Formatting">
            <ToolbarButton label="Bold" cmd="bold">
              <Bold size={13} />
            </ToolbarButton>
            <ToolbarButton label="Italic" cmd="italic">
              <Italic size={13} />
            </ToolbarButton>
            <ToolbarButton label="Underline" cmd="underline">
              <Underline size={13} />
            </ToolbarButton>
            <div className="mx-1 h-4 w-px bg-[var(--border)]" />
            <ToolbarButton label="Bulleted list" cmd="insertUnorderedList">
              <List size={13} />
            </ToolbarButton>
            <ToolbarButton label="Numbered list" cmd="insertOrderedList">
              <ListOrdered size={13} />
            </ToolbarButton>
            <ToolbarButton label="Quote" cmd="formatBlock" arg="blockquote">
              <Quote size={13} />
            </ToolbarButton>
            <ToolbarButton label="Insert link" cmd="createLink">
              <Link2 size={13} />
            </ToolbarButton>
            <div className="flex-1" />
            <IconButton label="Attach files" size="xs" onClick={() => fileRef.current?.click()}>
              <Paperclip size={13} />
            </IconButton>
            <input
              ref={fileRef}
              type="file"
              multiple
              hidden
              data-testid="composer-file-input"
              onChange={(e) => {
                void addFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </div>
          <div className="min-h-0 flex-1 cursor-text px-4 py-3" onClick={() => editorRef.current?.focus()}>
            <div
              ref={editorRef}
              role="textbox"
              aria-multiline="true"
              aria-label="Message body"
              data-testid="composer-editor"
              data-placeholder="Write your message…"
              contentEditable
              suppressContentEditableWarning
              data-autofocus={draft.to.length > 0 && !!draft.subject ? true : undefined}
              className="editor-content min-h-full"
            />
          </div>
          {draft.attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 border-t border-[var(--border-soft)] px-3 py-2.5">
              {draft.attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className="inline-flex h-7 items-center gap-2 rounded-lg bg-[var(--bg-input)] pl-2.5 pr-1 text-[12px] text-[var(--text-main)]"
                >
                  <Paperclip size={12} className="text-[var(--text-dim)]" />
                  {a.filename}
                  <span className="text-[var(--text-dim)]">{formatSize(Math.floor((a.dataBase64.length * 3) / 4))}</span>
                  <button
                    type="button"
                    aria-label={`Remove ${a.filename}`}
                    onClick={() => patch({ attachments: draft.attachments.filter((_, j) => j !== i) })}
                    className="rounded-md p-0.5 hover:bg-[var(--hover-bg)]"
                  >
                    <X size={12} />
                  </button>
                </span>
              ))}
            </div>
          )}
        </section>
      </ScrollArea>

      {/* Actions, pinned under the form */}
      <div className="shrink-0 border-t border-[var(--border-soft)] py-3">
        {error && <Alert className="mx-auto mb-3 max-w-[828px]">{error}</Alert>}
        <div className="mx-auto flex max-w-[860px] items-center gap-2 px-4">
          <Button variant="danger-ghost" icon={<Trash2 size={13} />} onClick={() => void discard()} aria-label="Discard">
            Discard
          </Button>
          <span className="flex-1" />
          <span className="hidden items-center gap-1 text-[11px] text-[var(--text-dim)] lg:flex">
            <Kbd>Ctrl</Kbd>
            <Kbd>↵</Kbd>
          </span>
          <Button icon={<Save size={13} />} onClick={() => void saveDraft()}>
            Save draft
          </Button>
          <Button variant="primary" icon={sending ? <Spinner size={13} /> : <Send size={13} />} disabled={sending} onClick={() => void send()}>
            Send
          </Button>
        </div>
      </div>
    </motion.section>
  );
}
