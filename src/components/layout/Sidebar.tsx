import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Archive,
  Check,
  ChevronRight,
  ChevronsUpDown,
  FilePen,
  Inbox,
  LoaderCircle,
  MoreHorizontal,
  OctagonAlert,
  PenSquare,
  Plus,
  Send,
  Settings,
  Tag as TagIcon,
  Trash2,
  UserCog,
} from "lucide-react";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { selectHasUpdate, useUpdateStore } from "@/store/updateStore";
import { api, errorMessage } from "@/lib/api";
import { emptyDraft } from "@/lib/compose";
import type { Account, Folder, FolderRole, Tag } from "@/lib/types";
import { SenderAvatar, VerifiedMark, domainOf } from "@/components/mail/SenderAvatar";
import { Input, Kbd, NavItem, Popover, ROW_ICON, RowActions, ScrollArea, cx } from "@/components";
import { TagDot, TagEditor } from "@/components/mail/Tags";

export const FOLDER_ICONS: Record<FolderRole, typeof Inbox> = {
  inbox: Inbox,
  sent: Send,
  drafts: FilePen,
  archive: Archive,
  spam: OctagonAlert,
  trash: Trash2,
  custom: TagIcon,
};

/** Primary mailboxes pinned to the top of the sidebar. */
const PRIMARY: FolderRole[] = ["inbox", "sent", "drafts", "archive"];

/** Drag payload types: a message row, or a tag being reordered. */
export const DRAG_EMAIL = "application/x-trajectory-email";
const DRAG_TAG = "application/x-trajectory-tag";

const WIDTH_KEY = "trajectory.sidebarWidth";
const LEGACY_WIDTH_KEY = "singularity.sidebarWidth";
/** Same default and limits as Gravitation's sidebar. */
const DEFAULT_W = 240;
const MIN_W = 220;
const MAX_W = 480;

function useSidebarWidth() {
  const [width, setWidth] = useState(() => {
    try {
      // The pre-rename key is dropped: the default width changed with it.
      localStorage.removeItem(LEGACY_WIDTH_KEY);
      const saved = Number(localStorage.getItem(WIDTH_KEY));
      if (saved >= MIN_W && saved <= MAX_W) return saved;
    } catch {
      /* storage unavailable */
    }
    return DEFAULT_W;
  });
  const startResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = width;
    let latest = startW;
    const onMove = (ev: MouseEvent) => {
      latest = Math.min(Math.max(startW + ev.clientX - startX, MIN_W), MAX_W);
      setWidth(latest);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      try {
        localStorage.setItem(WIDTH_KEY, String(latest));
      } catch {
        /* storage unavailable */
      }
    };
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };
  return { width, startResize };
}

/** A sidebar row with a label and a trailing count (unread / drafts). */
function Row({
  icon,
  label,
  count,
  active,
  testId,
  className,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  count?: number;
  active?: boolean;
  testId?: string;
  className?: string;
  onClick: () => void;
}) {
  return (
    <NavItem
      data-testid={testId}
      aria-current={active ? "page" : undefined}
      active={active}
      icon={icon}
      className={cx(active && "text-[var(--text-main)]", className)}
      onClick={onClick}
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {!!count && (
        <span className="ml-auto shrink-0 font-mono text-[11px] tabular-nums text-[var(--text-dim)]">{count > 999 ? "999+" : count}</span>
      )}
    </NavItem>
  );
}

function SectionHeader({
  label,
  open,
  onToggle,
  children,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div className="group mb-1 mt-3 flex h-6 shrink-0 items-center pl-1 pr-0.5">
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        title={open ? `Collapse ${label.toLowerCase()}` : `Expand ${label.toLowerCase()}`}
        className="flex h-6 items-center gap-1 rounded-md text-[12px] font-medium text-[var(--text-dim)] transition-colors hover:text-[var(--text-main)]"
      >
        {label}
        <motion.span
          animate={{ rotate: open ? 90 : 0 }}
          transition={{ duration: 0.15 }}
          className="flex items-center opacity-0 transition-opacity group-hover:opacity-100"
        >
          <ChevronRight size={12} strokeWidth={2} />
        </motion.span>
      </button>
      <span className="ml-auto flex items-center gap-2">{children}</span>
    </div>
  );
}

const HEADER_ICON = "flex items-center justify-center rounded-md p-0.5 text-[var(--text-muted)] opacity-60 transition-all hover:opacity-100";

function Collapse({ open, children }: { open: boolean; children: React.ReactNode }) {
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          className="flex shrink-0 flex-col gap-0.5 overflow-hidden"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: 0.18, ease: "easeOut" }}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function folderCount(f: Folder) {
  if (f.role === "drafts") return f.totalCount;
  if (f.role === "sent" || f.role === "trash" || f.role === "archive") return 0;
  return f.unreadCount;
}

/** Inbox unread of every account — fetched while the switcher is open. */
function useInboxUnread(accounts: Account[], enabled: boolean): Record<string, number> {
  const [unread, setUnread] = useState<Record<string, number>>({});
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void Promise.all(
      accounts.map(async (a) => [a.id, (await api.listFolders(a.id)).find((f) => f.role === "inbox")?.unreadCount ?? 0] as const),
    ).then((pairs) => alive && setUnread(Object.fromEntries(pairs)), () => {});
    return () => {
      alive = false;
    };
  }, [accounts, enabled]);
  return unread;
}

/**
 * The account in use, as a dropdown on top of the sidebar: switch mailbox,
 * add one, open account settings.
 */
function AccountSwitcher() {
  const accounts = useMailStore((s) => s.accounts);
  const activeId = useMailStore((s) => s.activeAccountId);
  const syncState = useMailStore((s) => s.syncState);
  const { selectAccount } = useMailStore.getState();
  const { openSettings } = useUiStore.getState();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const unread = useInboxUnread(accounts, open);
  const active = accounts.find((a) => a.id === activeId);
  if (!active) return null;

  const item = "flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] text-[var(--text-main)] transition-colors hover:bg-[var(--hover-bg)]";

  return (
    <>
      <button
        ref={trigger}
        type="button"
        data-testid="account-switcher"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Account: ${active.email}`}
        onClick={() => setOpen((o) => !o)}
        className={cx(
          "flex h-11 w-full items-center gap-2.5 rounded-xl px-2 text-left transition-colors hover:bg-[var(--hover-bg)]",
          open && "bg-[var(--hover-bg)]",
        )}
      >
        <SenderAvatar from={{ name: active.displayName, address: active.email }} logoDomain={domainOf(active.email)} color={active.color} size={26} />
        <span className="flex min-w-0 flex-1 flex-col leading-tight">
          <span className="flex min-w-0 items-center gap-1">
            <VerifiedMark logoDomain={domainOf(active.email)} size={12} />
            <span className="truncate text-[13px] font-medium text-[var(--text-main)]">{active.displayName || active.email}</span>
          </span>
          <span className="truncate text-[11.5px] text-[var(--text-dim)]">{active.email}</span>
        </span>
        {syncState[active.id] === "syncing" ? (
          <LoaderCircle size={13} strokeWidth={2} className="shrink-0 animate-spin text-[var(--accent)]" aria-label="Syncing…" />
        ) : syncState[active.id] === "error" ? (
          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--diff-del)]" aria-label="Sync error" />
        ) : null}
        <ChevronsUpDown size={14} className="shrink-0 text-[var(--text-dim)]" />
      </button>

      <Popover open={open} anchor={trigger} onClose={() => setOpen(false)} matchWidth label="Accounts">
        <div role="listbox" aria-label="Accounts" className="flex max-h-[320px] flex-col gap-0.5 overflow-y-auto">
          {accounts.map((a) => {
            const on = a.id === activeId;
            return (
              <button
                key={a.id}
                type="button"
                role="option"
                aria-selected={on}
                data-testid="account-option"
                title={a.email}
                onClick={() => {
                  setOpen(false);
                  if (!on) void selectAccount(a.id);
                }}
                className={cx("flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-[var(--hover-bg)]", on && "bg-[var(--hover-bg)]")}
              >
                <SenderAvatar from={{ name: a.displayName, address: a.email }} logoDomain={domainOf(a.email)} color={a.color} size={24} />
                <span className="flex min-w-0 flex-1 flex-col leading-tight">
                  <span className="flex min-w-0 items-center gap-1">
                    <VerifiedMark logoDomain={domainOf(a.email)} size={12} />
                    <span className="truncate text-[12.5px] text-[var(--text-main)]">{a.displayName || a.email}</span>
                  </span>
                  <span className="truncate text-[11px] text-[var(--text-dim)]">{a.email}</span>
                </span>
                {syncState[a.id] === "error" && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--diff-del)]" aria-label="Sync error" />}
                {!!unread[a.id] && <span className="shrink-0 font-mono text-[11px] tabular-nums text-[var(--text-dim)]">{unread[a.id]}</span>}
                <Check size={13} className={cx("shrink-0 text-[var(--accent)]", !on && "invisible")} />
              </button>
            );
          })}
        </div>
        <div className="my-1 h-px bg-[var(--border-soft)]" />
        <button
          type="button"
          className={item}
          onClick={() => {
            setOpen(false);
            openSettings("add");
          }}
        >
          <Plus size={14} strokeWidth={1.6} className="text-[var(--text-muted)]" />
          Add account
        </button>
        <button
          type="button"
          className={item}
          onClick={() => {
            setOpen(false);
            openSettings("accounts");
          }}
        >
          <UserCog size={14} strokeWidth={1.6} className="text-[var(--text-muted)]" />
          Manage accounts
        </button>
      </Popover>
    </>
  );
}

/**
 * A tag in the sidebar: opens its message list; drag it to reorder, drop
 * messages on it to tag them; ⋯ edits it.
 */
function TagRow({
  tag,
  index,
  active,
  dropHint,
  onDragOverRow,
  onDropRow,
  onDragEnd,
}: {
  tag: Tag;
  index: number;
  active: boolean;
  /** Reorder indicator: line above or below this row. */
  dropHint: "above" | "below" | "into" | null;
  onDragOverRow: (e: React.DragEvent, index: number) => void;
  onDropRow: (e: React.DragEvent, index: number) => void;
  onDragEnd: () => void;
}) {
  const { selectTag } = useMailStore.getState();
  const [hovered, setHovered] = useState(false);
  const [editing, setEditing] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);

  return (
    <div
      className="relative shrink-0"
      data-testid="tag-row"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_TAG, tag.id);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => onDragOverRow(e, index)}
      onDrop={(e) => onDropRow(e, index)}
      onDragEnd={onDragEnd}
    >
      {dropHint === "above" && <span className="pointer-events-none absolute -top-px left-2 right-2 h-0.5 rounded-full bg-[var(--accent)]" />}
      {dropHint === "below" && <span className="pointer-events-none absolute -bottom-px left-2 right-2 h-0.5 rounded-full bg-[var(--accent)]" />}
      <NavItem
        aria-current={active ? "page" : undefined}
        active={active}
        hovered={!active && (hovered || editing || dropHint === "into")}
        className={cx(active && "text-[var(--text-main)]", dropHint === "into" && "ring-1 ring-[var(--accent)]")}
        icon={<TagDot color={tag.color} className="mx-[3px]" />}
        onClick={() => void selectTag(tag.id)}
        title={index < 9 ? `${tag.name} — press ${index + 1} to tag the open message` : tag.name}
      >
        <span className="min-w-0 flex-1 truncate">{tag.name}</span>
        {!!tag.count && <span className="ml-auto shrink-0 font-mono text-[11px] tabular-nums text-[var(--text-dim)]">{tag.count}</span>}
      </NavItem>
      <RowActions show={hovered || editing}>
        <button
          ref={anchor}
          type="button"
          className={ROW_ICON}
          title="Edit tag"
          aria-label={`Edit tag ${tag.name}`}
          onClick={(e) => {
            e.stopPropagation();
            setEditing((o) => !o);
          }}
        >
          <MoreHorizontal size={14} strokeWidth={1.5} />
        </button>
      </RowActions>
      <TagEditor tag={tag} open={editing} anchor={anchor} onClose={() => setEditing(false)} />
    </div>
  );
}

function TagsSection() {
  const tags = useMailStore((s) => s.tags);
  const activeTagId = useMailStore((s) => s.activeTagId);
  const { createTag, moveTag, tagEmail } = useMailStore.getState();
  const pushToast = useUiStore((s) => s.pushToast);
  const [open, setOpen] = useState(true);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [hint, setHint] = useState<{ index: number; where: "above" | "below" | "into" } | null>(null);

  const add = async () => {
    if (!name.trim()) return setAdding(false);
    try {
      await createTag(name);
      setName("");
      setAdding(false);
    } catch (e) {
      pushToast(errorMessage(e), "error");
    }
  };

  const onDragOverRow = (e: React.DragEvent, index: number) => {
    const types = e.dataTransfer.types;
    if (types.includes(DRAG_EMAIL)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setHint({ index, where: "into" });
    } else if (types.includes(DRAG_TAG)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const r = e.currentTarget.getBoundingClientRect();
      setHint({ index, where: e.clientY < r.top + r.height / 2 ? "above" : "below" });
    }
  };

  const onDropRow = (e: React.DragEvent, index: number) => {
    e.preventDefault();
    const where = hint?.where;
    setHint(null);
    const emailId = e.dataTransfer.getData(DRAG_EMAIL);
    if (emailId) {
      const tag = tags[index];
      void tagEmail(emailId, tag.id, true).then(() => pushToast(`Tagged “${tag.name}”`, "success"), () => {});
      return;
    }
    const tagId = e.dataTransfer.getData(DRAG_TAG);
    if (!tagId) return;
    const from = tags.findIndex((t) => t.id === tagId);
    let to = where === "below" ? index + 1 : index;
    if (from < to) to -= 1;
    void moveTag(tagId, to);
  };

  return (
    <>
      <SectionHeader label="Tags" open={open} onToggle={() => setOpen((o) => !o)}>
        <button
          type="button"
          className={HEADER_ICON}
          title="New tag"
          aria-label="New tag"
          onClick={() => {
            setOpen(true);
            setAdding(true);
          }}
        >
          <Plus size={14} strokeWidth={1.5} />
        </button>
      </SectionHeader>
      <Collapse open={open}>
        <div className="flex flex-col gap-0.5" onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setHint(null)}>
          {tags.map((t, i) => (
            <TagRow
              key={t.id}
              tag={t}
              index={i}
              active={t.id === activeTagId}
              dropHint={hint?.index === i ? hint.where : null}
              onDragOverRow={onDragOverRow}
              onDropRow={onDropRow}
              onDragEnd={() => setHint(null)}
            />
          ))}
        </div>
        {adding ? (
          <form
            className="px-0.5 py-0.5"
            onSubmit={(e) => {
              e.preventDefault();
              void add();
            }}
          >
            <Input
              size="sm"
              autoFocus
              aria-label="New tag name"
              placeholder="Tag name"
              maxLength={40}
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => void add()}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.stopPropagation();
                  setName("");
                  setAdding(false);
                }
              }}
            />
          </form>
        ) : (
          tags.length === 0 && (
            <button
              type="button"
              onClick={() => setAdding(true)}
              className="flex h-7 items-center gap-2 rounded-lg px-2.5 text-left text-[12px] text-[var(--text-dim)] transition-colors hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]"
            >
              <Plus size={13} strokeWidth={1.6} />
              Create a tag to sort mail
            </button>
          )
        )}
      </Collapse>
    </>
  );
}

export function Sidebar() {
  const { width, startResize } = useSidebarWidth();
  const folders = useMailStore((s) => s.folders);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeTagId = useMailStore((s) => s.activeTagId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const { selectFolder } = useMailStore.getState();
  const { openSettings, openComposer } = useUiStore.getState();
  const hasUpdate = useUpdateStore(selectHasUpdate);
  const updateVersion = useUpdateStore((s) => s.release?.version);
  const [foldersOpen, setFoldersOpen] = useState(true);

  const primary = PRIMARY.map((role) => folders.find((f) => f.role === role)).filter((f): f is Folder => !!f);
  const others = folders.filter((f) => !primary.includes(f));
  const folderActive = (f: Folder) => !activeTagId && f.id === activeFolderId;

  return (
    <aside
      aria-label="Mailboxes"
      className="relative flex h-full shrink-0 flex-col bg-[var(--bg-sidebar)] text-[13px] leading-tight"
      style={{ width }}
    >
      {/* Header: the account in use, compose, its primary mailboxes */}
      <div className="flex flex-col gap-2 px-2.5 pt-2.5">
        <AccountSwitcher />
        <nav className="flex flex-col gap-0.5" aria-label="Folders">
          <NavItem
            icon={<PenSquare size={16} strokeWidth={1.5} className="shrink-0" />}
            disabled={!activeAccountId}
            className="mb-1.5 disabled:opacity-50"
            onClick={() => activeAccountId && openComposer(emptyDraft(activeAccountId))}
          >
            <span className="flex-1">New message</span>
            <Kbd>C</Kbd>
          </NavItem>
          {primary.map((f) => {
            const Icon = FOLDER_ICONS[f.role];
            return (
              <Row
                key={f.id}
                testId={`folder-${f.role}`}
                icon={<Icon size={16} strokeWidth={1.5} className="shrink-0" />}
                label={f.name}
                count={folderCount(f)}
                active={folderActive(f)}
                onClick={() => void selectFolder(f.id)}
              />
            );
          })}
        </nav>
      </div>

      {/* Other folders and the user's tags share one scroll region */}
      <ScrollArea className="min-h-0 flex-1" innerClassName="flex flex-col gap-0.5 px-2.5 pt-1 [&>*]:shrink-0">
        {others.length > 0 && (
          <>
            <SectionHeader label="Folders" open={foldersOpen} onToggle={() => setFoldersOpen((o) => !o)} />
            <Collapse open={foldersOpen}>
              {others.map((f) => {
                const Icon = FOLDER_ICONS[f.role];
                return (
                  <Row
                    key={f.id}
                    testId={`folder-${f.role}`}
                    icon={<Icon size={15} strokeWidth={1.5} className="shrink-0" />}
                    label={f.name}
                    count={folderCount(f)}
                    active={folderActive(f)}
                    onClick={() => void selectFolder(f.id)}
                  />
                );
              })}
            </Collapse>
          </>
        )}
        <TagsSection />
        <div className="h-2 shrink-0" />
      </ScrollArea>

      {/* Footer — Settings pinned to the very bottom */}
      <div className="mt-auto shrink-0 px-2.5 pb-3 pt-1">
        <NavItem
          data-testid="open-settings"
          icon={<Settings size={16} strokeWidth={1.5} className="shrink-0" />}
          onClick={() => openSettings("accounts")}
        >
          <span>Settings</span>
          {hasUpdate && (
            <span
              role="button"
              data-testid="update-pill"
              className="ml-auto rounded-full bg-[var(--accent)]/15 px-2 py-0.5 text-[11px] font-medium text-[var(--accent)] hover:bg-[var(--accent)]/25"
              title={`Version ${updateVersion} is available`}
              onClick={(e) => {
                e.stopPropagation();
                openSettings("about");
              }}
            >
              Update
            </span>
          )}
        </NavItem>
      </div>

      <div className="sidebar-resizer" onMouseDown={startResize} role="separator" aria-orientation="vertical" aria-label="Resize sidebar" />
    </aside>
  );
}
