import { useState } from "react";
import { cx } from "../cx.u";
import { avatarColor, initials } from "@/lib/format";

/** Round avatar: a logo when one is given, otherwise initials in a colour stable per address. */
export function Avatar({
  name,
  email,
  size = 32,
  color,
  src,
}: {
  name: string;
  email?: string;
  size?: number;
  color?: string;
  /** Logo image (a data: URL). Falls back to initials if it fails to load. */
  src?: string | null;
}) {
  const [broken, setBroken] = useState<string | null>(null);
  const c = color ?? avatarColor(email ?? name);
  const logo = src && src !== broken ? src : null;
  return (
    <span aria-hidden="true" className="relative inline-flex shrink-0 select-none" style={{ width: size, height: size }}>
      {logo ? (
        <img
          src={logo}
          alt=""
          draggable={false}
          data-testid="sender-logo"
          onError={() => setBroken(logo)}
          className="h-full w-full rounded-full bg-[var(--bg-sidebar)] object-cover ring-1 ring-[var(--border-soft)]"
        />
      ) : (
        <span
          className="inline-flex h-full w-full items-center justify-center rounded-full font-semibold text-white"
          style={{ fontSize: Math.max(10, size * 0.38), background: c }}
        >
          {initials(name || email || "?")}
        </span>
      )}
    </span>
  );
}

export type PillTone = "neutral" | "accent" | "success" | "danger" | "warning";

const PILL: Record<PillTone, string> = {
  neutral: "bg-[var(--hover-bg)] text-[var(--text-muted)]",
  accent: "bg-[var(--accent)]/15 text-[var(--accent)]",
  success: "bg-[var(--diff-add)]/15 text-[var(--diff-add)]",
  danger: "bg-[var(--diff-del)]/15 text-[var(--diff-del)]",
  warning: "bg-amber-500/15 text-amber-500",
};

/** Small status pill (provider, sync error). */
export function Pill({ tone = "neutral", children, className }: { tone?: PillTone; children: React.ReactNode; className?: string }) {
  return (
    <span className={cx("inline-flex h-5 shrink-0 items-center gap-1 rounded-md px-1.5 text-[10.5px] font-medium", PILL[tone], className)}>
      {children}
    </span>
  );
}

/** Keyboard key hint. */
export function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-md bg-[var(--bg-input)] px-1 font-sans text-[10px] text-[var(--text-dim)]">
      {children}
    </kbd>
  );
}
