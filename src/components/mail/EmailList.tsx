import { memo, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Archive, ArrowDownUp, Check, Flag, Mail, MailOpen, Paperclip, Search, StickyNote, Trash2, X } from "lucide-react";
import { hasMailFilters, useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { draftFromEmail } from "@/lib/compose";
import { api } from "@/lib/api";
import { displayName, formatListDate } from "@/lib/format";
import type { EmailSummary, MailFilter, MailSort, Tag } from "@/lib/types";
import { ContextMenu, IconButton, Input, OverlayScroll, Popover, Segmented, Spinner, cx, type ContextMenuItem } from "@/components";
import { DRAG_EMAIL } from "@/components/layout/Sidebar";
import { TagChip, TagDot } from "./Tags";
import { SenderAvatar, VerifiedMark } from "./SenderAvatar";

export const ROW_HEIGHT = 92;

const FILTERS: { value: MailFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "unread", label: "Unread" },
  { value: "flagged", label: "Flagged" },
];

export const SORT_OPTIONS: { value: MailSort; label: string }[] = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "unread", label: "Unread first" },
  { value: "flagged", label: "Flagged first" },
  { value: "sender", label: "Sender A–Z" },
  { value: "subject", label: "Subject A–Z" },
];

/** Up to two chips, then "+n". */
const MAX_CHIPS = 2;

export const EmailListItem = memo(function EmailListItem({
  email,
  tags,
  selected,
  onSelect,
  onToggleFlag,
  onContextMenu,
}: {
  email: EmailSummary;
  /** The message's tags, already resolved and ordered. */
  tags: Tag[];
  selected: boolean;
  onSelect: (id: string) => void;
  onToggleFlag: (id: string) => void;
  onContextMenu?: (e: React.MouseEvent, email: EmailSummary) => void;
}) {
  const sender = displayName(email.from);
  return (
    <div
      role="option"
      aria-selected={selected}
      data-testid="email-row"
      data-unread={!email.isRead || undefined}
      tabIndex={-1}
      draggable
      onDragStart={(e) => {
        // Dropped on a tag in the sidebar = tag the message.
        e.dataTransfer.setData(DRAG_EMAIL, email.id);
        e.dataTransfer.effectAllowed = "copy";
      }}
      onClick={() => onSelect(email.id)}
      onContextMenu={onContextMenu && ((e) => onContextMenu(e, email))}
      className={cx(
        "group relative flex h-[84px] cursor-pointer gap-3 rounded-xl px-3 py-2.5 transition-colors duration-100",
        selected ? "bg-[var(--hover-bg)]" : "hover:bg-[var(--hover-bg)]",
      )}
    >
      {!email.isRead && <span aria-label="Unread" className="absolute left-1 top-[22px] h-1.5 w-1.5 rounded-full bg-[var(--accent)]" />}
      <SenderAvatar from={email.from} logoDomain={email.logoDomain} size={32} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex items-center gap-2">
          <span className="flex min-w-0 flex-1 items-center gap-1">
            <VerifiedMark logoDomain={email.logoDomain} />
            <span className={cx("truncate text-[13px]", email.isRead ? "text-[var(--text-muted)]" : "font-semibold text-[var(--text-main)]")}>{sender}</span>
          </span>
          {email.hasAttachments && <Paperclip size={12} className="shrink-0 text-[var(--text-dim)]" aria-label="Has attachments" />}
          <time className="shrink-0 font-mono text-[11px] tabular-nums text-[var(--text-dim)]" dateTime={new Date(email.date * 1000).toISOString()}>
            {formatListDate(email.date)}
          </time>
        </div>
        <div className="flex items-center gap-2">
          <span className={cx("flex-1 truncate text-[13px]", email.isRead ? "text-[var(--text-muted)]" : "text-[var(--text-main)]")}>
            {email.subject || "(no subject)"}
          </span>
          <button
            type="button"
            aria-label={email.isFlagged ? "Unflag" : "Flag"}
            aria-pressed={email.isFlagged}
            onClick={(e) => {
              e.stopPropagation();
              onToggleFlag(email.id);
            }}
            className={cx(
              "shrink-0 rounded-lg p-0.5 transition-opacity",
              email.isFlagged ? "text-amber-500" : "text-[var(--text-dim)] opacity-0 hover:text-[var(--text-main)] group-hover:opacity-100",
            )}
          >
            <Flag size={13} fill={email.isFlagged ? "currentColor" : "none"} />
          </button>
        </div>
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="min-w-0 flex-1 truncate text-[12px] text-[var(--text-dim)]">{email.snippet}</p>
          {email.hasNote && <StickyNote size={12} className="shrink-0 text-[var(--text-dim)]" aria-label="Has a note" />}
          {tags.slice(0, MAX_CHIPS).map((t) => (
            <TagChip key={t.id} tag={t} size="sm" />
          ))}
          {tags.length > MAX_CHIPS && (
            <span className="shrink-0 text-[10.5px] text-[var(--text-dim)]" title={tags.slice(MAX_CHIPS).map((t) => t.name).join(", ")}>
              +{tags.length - MAX_CHIPS}
            </span>
          )}
        </div>
      </div>
    </div>
  );
});

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** "Newest first ▾" — how the list is ordered. */
function SortMenu() {
  const sort = useMailStore((s) => s.sort);
  const { setSort } = useMailStore.getState();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const label = SORT_OPTIONS.find((o) => o.value === sort)?.label ?? "Newest first";
  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Sort: ${label}`}
        onClick={() => setOpen((o) => !o)}
        className={cx(
          "flex h-6 items-center gap-1 rounded-md px-1.5 text-[11.5px] text-[var(--text-dim)] transition-colors hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]",
          open && "bg-[var(--hover-bg)] text-[var(--text-main)]",
        )}
      >
        <ArrowDownUp size={12} />
        {label}
      </button>
      <Popover open={open} anchor={anchor} onClose={() => setOpen(false)} align="end" label="Sort messages" className="min-w-[180px]">
        <div role="radiogroup" aria-label="Sort messages" className="flex flex-col gap-0.5">
          {SORT_OPTIONS.map((o) => (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={o.value === sort}
              onClick={() => {
                setOpen(false);
                void setSort(o.value);
              }}
              className="flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[13px] text-[var(--text-main)] transition-colors hover:bg-[var(--hover-bg)]"
            >
              <span className="flex-1">{o.label}</span>
              <Check size={13} className={cx("text-[var(--accent)]", o.value !== sort && "invisible")} />
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}

export function EmailList() {
  const emails = useMailStore((s) => s.emails);
  const total = useMailStore((s) => s.total);
  const filter = useMailStore((s) => s.filter);
  const loading = useMailStore((s) => s.loadingList);
  const hasMore = useMailStore((s) => s.hasMore);
  const older = useMailStore((s) => (s.activeFolderId ? s.olderState[s.activeFolderId] : undefined));
  const selectedId = useMailStore((s) => s.selectedEmailId);
  const folder = useMailStore((s) => s.folders.find((f) => f.id === s.activeFolderId));
  const tags = useMailStore((s) => s.tags);
  const activeTag = useMailStore((s) => s.tags.find((t) => t.id === s.activeTagId));
  const { setFilter, setSearch, selectEmail, toggleFlag, loadMore, loadOlder, markRead, tagEmail, archive, remove } = useMailStore.getState();
  const openComposer = useUiStore((s) => s.openComposer);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; email: EmailSummary } | null>(null);
  const tagsOf = (e: EmailSummary) => (e.tags.length ? tags.filter((t) => e.tags.includes(t.id)) : NO_TAGS);

  const [query, setQuery] = useState(useMailStore.getState().search);
  const debounced = useDebounced(query, 250);
  useEffect(() => {
    if (debounced !== useMailStore.getState().search) void setSearch(debounced);
  }, [debounced, setSearch]);

  const parentRef = useRef<HTMLDivElement>(null!);
  const virtualizer = useVirtualizer({
    count: emails.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
    initialRect: { width: 360, height: 900 },
  });
  const items = virtualizer.getVirtualItems();

  // Infinite scroll: fetch the next page when the last rows become visible.
  const lastIndex = items.length ? items[items.length - 1].index : -1;
  // Past what is stored locally, older mail comes from the server — only when
  // the list is actually scrolled to its end, never all at once.
  const scrolledToEnd = emails.length > 0 && lastIndex >= emails.length - 5;
  useEffect(() => {
    if (loading || !scrolledToEnd) return;
    if (hasMore) void loadMore();
    else if (!older) void loadOlder();
  }, [scrolledToEnd, hasMore, loading, older, emails.length, loadMore, loadOlder]);

  // Keep the selected row visible during keyboard navigation.
  useEffect(() => {
    const idx = emails.findIndex((e) => e.id === selectedId);
    if (idx >= 0) virtualizer.scrollToIndex(idx, { align: "auto" });
  }, [selectedId, emails, virtualizer]);

  const onContextMenu = (e: React.MouseEvent, email: EmailSummary) => {
    e.preventDefault();
    setMenu({ at: { x: e.clientX, y: e.clientY }, email });
  };
  // Re-read the row from the store so the menu shows its current state.
  const menuEmail = menu && (emails.find((e) => e.id === menu.email.id) ?? menu.email);
  const menuItems: ContextMenuItem[] = menuEmail
    ? [
        {
          icon: menuEmail.isRead ? <Mail size={14} /> : <MailOpen size={14} />,
          label: menuEmail.isRead ? "Mark as unread" : "Mark as read",
          hint: "U",
          onClick: () => void markRead(menuEmail.id, !menuEmail.isRead),
        },
        {
          icon: <Flag size={14} />,
          label: menuEmail.isFlagged ? "Unflag" : "Flag",
          hint: "S",
          onClick: () => void toggleFlag(menuEmail.id),
        },
        ...(tags.length ? (["separator"] as const) : []),
        ...tags.map(
          (t, i): ContextMenuItem => ({
            icon: menuEmail.tags.includes(t.id) ? <Check size={14} style={{ color: t.color }} /> : <TagDot color={t.color} />,
            label: t.name,
            hint: i < 9 ? String(i + 1) : undefined,
            onClick: () => void tagEmail(menuEmail.id, t.id),
          }),
        ),
        "separator",
        { icon: <Archive size={14} />, label: "Archive", hint: "E", onClick: () => void archive(menuEmail.id) },
        { icon: <Trash2 size={14} />, label: "Delete", danger: true, onClick: () => void remove(menuEmail.id) },
      ]
    : [];

  const onSelect = async (id: string) => {
    if (folder?.role === "drafts") {
      const draft = draftFromEmail(await api.getEmail(id));
      if (useUiStore.getState().composer?.draftId === id) return;
      openComposer(draft);
      return;
    }
    void selectEmail(id);
  };

  return (
    <section className="flex h-full w-[380px] shrink-0 flex-col border-r border-[var(--border-soft)]" aria-label="Messages">
      <header className="flex flex-col gap-2.5 px-3 pb-2 pt-3">
        <div className="flex h-7 items-center justify-between px-1">
          <h1 className="flex min-w-0 items-center gap-2 text-[15px] font-semibold text-[var(--text-main)]">
            {activeTag && <TagDot color={activeTag.color} size={9} />}
            <span className="truncate">{activeTag?.name ?? folder?.name ?? "Mail"}</span>
          </h1>
          <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-[var(--text-dim)]">
            {loading && <Spinner size={12} />}
            <span>
              {total} {total === 1 ? "message" : "messages"}
            </span>
            <SortMenu />
          </span>
        </div>
        <div className="relative">
          <Search size={13} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
          <Input
            id="mail-search"
            placeholder="Search mail"
            aria-label="Search mail"
            className="pl-8 pr-8"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {query && (
            <IconButton label="Clear search" size="xs" className="absolute right-1.5 top-1/2 -translate-y-1/2" onClick={() => setQuery("")}>
              <X size={12} />
            </IconButton>
          )}
        </div>
        {hasMailFilters(folder, activeTag?.id ?? null) && (
          <Segmented<MailFilter> fill size="sm" value={filter} onChange={(f) => void setFilter(f)} options={FILTERS} />
        )}
      </header>

      <OverlayScroll innerRef={parentRef} wrapperClassName="min-h-0 flex-1" className="h-full overflow-y-auto px-2 pb-2">
      <div role="listbox" aria-label="Email list" className="relative h-full">
        {emails.length === 0 && !loading ? (
          <EmptyList
            title={query ? "No results" : filter !== "all" ? `No ${filter} messages` : activeTag ? `Nothing tagged “${activeTag.name}”` : "Nothing here"}
            text={
              query
                ? "Try a different search."
                : activeTag
                  ? "Drag a message onto the tag, right-click it, or press T while reading."
                  : "You're all caught up."
            }
          />
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {items.map((row) => {
              const email = emails[row.index];
              return (
                <div
                  key={email.id}
                  style={{ position: "absolute", top: 0, left: 0, right: 0, height: row.size, transform: `translateY(${row.start}px)` }}
                  className="py-1"
                >
                  <EmailListItem
                    email={email}
                    tags={tagsOf(email)}
                    selected={email.id === selectedId}
                    onSelect={onSelect}
                    onToggleFlag={toggleFlag}
                    onContextMenu={onContextMenu}
                  />
                </div>
              );
            })}
          </div>
        )}
        {older === "loading" && (
          <div role="status" className="flex items-center justify-center gap-2 py-3 text-[12px] text-[var(--text-dim)]">
            <Spinner size={13} /> Loading older messages…
          </div>
        )}
      </div>
      </OverlayScroll>
      <ContextMenu at={menu?.at ?? null} items={menuItems} onClose={() => setMenu(null)} />
    </section>
  );
}

/** Shared empty value, so untagged rows keep a stable prop for `memo`. */
const NO_TAGS: Tag[] = [];

/** Bar widths of the ghost rows (sender, subject). */
const GHOST: [number, number][] = [
  [34, 70],
  [26, 58],
  [40, 76],
];

/** Empty folder / no results: a quiet title over ghost rows fading out. */
function EmptyList({ title, text }: { title: string; text: string }) {
  return (
    <div className="flex flex-col items-center pt-10">
      <div className="text-[13px] font-medium text-[var(--text-main)]">{title}</div>
      <div className="mt-1 text-[12px] text-[var(--text-dim)]">{text}</div>
      <div
        aria-hidden
        className="mt-6 flex w-full select-none flex-col gap-0.5"
        style={{ maskImage: "linear-gradient(to bottom, black 10%, transparent 95%)" }}
      >
        {GHOST.map(([from, subject], i) => (
          <div key={i} className="flex items-center gap-3 px-3 py-2.5">
            <span className="h-8 w-8 shrink-0 rounded-full bg-[var(--bg-input)]" />
            <span className="flex min-w-0 flex-1 flex-col gap-1.5">
              <span className="h-2.5 rounded-full bg-[var(--bg-input)]" style={{ width: from + "%" }} />
              <span className="h-2 rounded-full bg-[var(--bg-input)] opacity-70" style={{ width: subject + "%" }} />
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
