import { useEffect, useMemo, useState } from "react";
import { Check, Plus, Search, Trash2, X } from "lucide-react";
import { useMailStore, TAG_COLORS } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { errorMessage } from "@/lib/api";
import type { Tag } from "@/lib/types";
import { Button, Input, Kbd, Popover, cx } from "@/components";

/** The colour mark of a tag. */
export function TagDot({ color, size = 8, className }: { color: string; size?: number; className?: string }) {
  return <span aria-hidden className={cx("shrink-0 rounded-full", className)} style={{ width: size, height: size, background: color }} />;
}

/** A tag on a message: tinted pill with its colour; optional remove button. */
export function TagChip({ tag, onRemove, size = "md" }: { tag: Tag; onRemove?: () => void; size?: "sm" | "md" }) {
  return (
    <span
      data-testid="tag-chip"
      className={cx(
        "inline-flex max-w-[160px] shrink-0 items-center gap-1.5 rounded-full border font-medium",
        size === "sm" ? "h-[18px] px-1.5 text-[10.5px]" : "h-6 px-2.5 text-[11.5px]",
      )}
      style={{ color: tag.color, borderColor: `${tag.color}55`, background: `${tag.color}1a` }}
    >
      <TagDot color={tag.color} size={size === "sm" ? 5 : 6} />
      <span className="truncate">{tag.name}</span>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove tag ${tag.name}`}
          onClick={onRemove}
          className="-mr-1 rounded-full p-0.5 opacity-70 transition-opacity hover:opacity-100"
        >
          <X size={11} />
        </button>
      )}
    </span>
  );
}

/** Tags of a message, in the user's tag order. */
export function useTagsOf(ids: string[]): Tag[] {
  const tags = useMailStore((s) => s.tags);
  return useMemo(() => tags.filter((t) => ids.includes(t.id)), [tags, ids]);
}

/**
 * Picks the tags of a message: a filter box that doubles as "create", a
 * checkable list (1–9 also work as shortcuts while it is closed), Enter on a
 * new name creates the tag and puts it on the message.
 */
export function TagPicker({
  emailId,
  selected,
  open,
  anchor,
  onClose,
}: {
  emailId: string;
  selected: string[];
  open: boolean;
  anchor: React.RefObject<HTMLElement | null>;
  onClose: () => void;
}) {
  const tags = useMailStore((s) => s.tags);
  const { tagEmail, createTag } = useMailStore.getState();
  const pushToast = useUiStore((s) => s.pushToast);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setCursor(0);
  }, [open]);

  const q = query.trim().toLowerCase();
  const shown = q ? tags.filter((t) => t.name.toLowerCase().includes(q)) : tags;
  const canCreate = !!q && !tags.some((t) => t.name.toLowerCase() === q);
  const rows = shown.length + (canCreate ? 1 : 0);

  const create = async () => {
    try {
      const tag = await createTag(query);
      await tagEmail(emailId, tag.id, true);
      setQuery("");
    } catch (e) {
      pushToast(errorMessage(e), "error");
    }
  };
  const pick = (i: number) => {
    if (i < shown.length) void tagEmail(emailId, shown[i].id);
    else if (canCreate) void create();
  };

  return (
    <Popover open={open} anchor={anchor} onClose={onClose} align="end" label="Tags" className="w-[260px]">
      <div className="relative mb-1">
        <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
        <Input
          autoFocus
          size="sm"
          aria-label="Find or create a tag"
          placeholder="Find or create a tag…"
          className="pl-8"
          value={query}
          maxLength={40}
          onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setCursor((c) => Math.min(rows - 1, c + 1));
            else if (e.key === "ArrowUp") setCursor((c) => Math.max(0, c - 1));
            else if (e.key === "Enter") pick(cursor);
            else return;
            e.preventDefault();
          }}
        />
      </div>
      <div role="listbox" aria-label="Tags" aria-multiselectable className="flex max-h-[280px] flex-col gap-0.5 overflow-y-auto">
        {shown.map((t, i) => {
          const on = selected.includes(t.id);
          const index = tags.indexOf(t);
          return (
            <button
              key={t.id}
              type="button"
              role="option"
              aria-selected={on}
              onMouseEnter={() => setCursor(i)}
              onClick={() => pick(i)}
              className={cx(
                "flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[13px] text-[var(--text-main)]",
                cursor === i && "bg-[var(--hover-bg)]",
              )}
            >
              <TagDot color={t.color} />
              <span className="min-w-0 flex-1 truncate">{t.name}</span>
              {index < 9 && <Kbd>{index + 1}</Kbd>}
              <Check size={13} className={cx("shrink-0 text-[var(--accent)]", !on && "invisible")} />
            </button>
          );
        })}
        {canCreate && (
          <button
            type="button"
            role="option"
            aria-selected={false}
            onMouseEnter={() => setCursor(shown.length)}
            onClick={() => void create()}
            className={cx(
              "flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[13px] text-[var(--accent)]",
              cursor === shown.length && "bg-[var(--hover-bg)]",
            )}
          >
            <Plus size={13} className="shrink-0" />
            <span className="min-w-0 flex-1 truncate">Create “{query.trim()}”</span>
          </button>
        )}
        {rows === 0 && <div className="px-2.5 py-2 text-[12px] text-[var(--text-dim)]">Type a name to create your first tag.</div>}
      </div>
    </Popover>
  );
}

/** Rename / recolour / delete a tag (from the sidebar). */
export function TagEditor({
  tag,
  open,
  anchor,
  onClose,
}: {
  tag: Tag;
  open: boolean;
  anchor: React.RefObject<HTMLElement | null>;
  onClose: () => void;
}) {
  const { updateTag, deleteTag } = useMailStore.getState();
  const pushToast = useUiStore((s) => s.pushToast);
  const [name, setName] = useState(tag.name);
  const [color, setColor] = useState(tag.color);
  const [confirm, setConfirm] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(tag.name);
    setColor(tag.color);
    setConfirm(false);
  }, [open, tag.name, tag.color]);

  const save = async () => {
    try {
      if (name.trim() !== tag.name || color !== tag.color) await updateTag(tag.id, name, color);
      onClose();
    } catch (e) {
      pushToast(errorMessage(e), "error");
    }
  };

  return (
    <Popover open={open} anchor={anchor} onClose={onClose} label={`Edit tag ${tag.name}`} className="w-[268px] gap-2.5 p-2.5">
      <form
        className="flex flex-col gap-2.5"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Input size="sm" aria-label="Tag name" value={name} maxLength={40} autoFocus onChange={(e) => setName(e.target.value)} />
        <div className="flex flex-wrap gap-1.5" role="radiogroup" aria-label="Tag colour">
          {TAG_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              role="radio"
              aria-checked={c === color}
              aria-label={c}
              onClick={() => setColor(c)}
              className={cx(
                "flex h-6 w-6 items-center justify-center rounded-full transition-transform hover:scale-110",
                c === color && "ring-2 ring-[var(--text-main)] ring-offset-2 ring-offset-[var(--bg-surface)]",
              )}
              style={{ background: c }}
            />
          ))}
        </div>
        <div className="flex items-center gap-1.5">
          {confirm ? (
            <>
              <span className="flex-1 text-[12px] text-[var(--text-muted)]">Delete for all messages?</span>
              <Button size="xs" onClick={() => setConfirm(false)}>
                Cancel
              </Button>
              <Button
                size="xs"
                variant="danger"
                onClick={() =>
                  void deleteTag(tag.id).then(onClose, (e) => pushToast(errorMessage(e), "error"))
                }
              >
                Delete
              </Button>
            </>
          ) : (
            <>
              <Button size="xs" variant="danger-ghost" icon={<Trash2 size={12} />} onClick={() => setConfirm(true)}>
                Delete
              </Button>
              <div className="flex-1" />
              <Button size="xs" type="submit" variant="primary" disabled={!name.trim()}>
                Save
              </Button>
            </>
          )}
        </div>
      </form>
    </Popover>
  );
}
