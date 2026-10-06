import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "motion/react";
import { cx } from "../cx.u";

/**
 * A floating panel under a trigger — RowMenu's look, but with any content
 * (account switcher, tag picker). Fixed-positioned against the trigger, so no
 * scroll container clips it; flips above when there is no room below. Closes
 * on outside click and Escape.
 */
export function Popover({
  open,
  anchor,
  onClose,
  align = "start",
  matchWidth = false,
  label,
  className,
  children,
}: {
  open: boolean;
  anchor: React.RefObject<HTMLElement | null>;
  onClose: () => void;
  /** Which edge of the trigger the panel lines up with. */
  align?: "start" | "end";
  /** As wide as the trigger. */
  matchWidth?: boolean;
  /** Accessible name of the panel. */
  label?: string;
  className?: string;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number; width?: number; flipped: boolean }>({ top: 0, left: 0, flipped: false });
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const el = anchor.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const w = matchWidth ? r.width : (ref.current?.offsetWidth ?? 240);
      const h = ref.current?.offsetHeight ?? 200;
      const gap = 6;
      const fitBelow = r.bottom + gap + h <= window.innerHeight - 8;
      const top = fitBelow ? r.bottom + gap : Math.max(8, r.top - gap - h);
      const raw = align === "start" ? r.left : r.right - w;
      const left = Math.min(Math.max(8, raw), window.innerWidth - w - 8);
      setPos((p) =>
        p.top === top && p.left === left && p.width === (matchWidth ? w : undefined) && p.flipped === !fitBelow
          ? p
          : { top, left, width: matchWidth ? w : undefined, flipped: !fitBelow },
      );
    };
    place();
    const raf = requestAnimationFrame(place);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open, anchor, align, matchWidth]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (ref.current?.contains(t) || anchor.current?.contains(t)) return;
      onCloseRef.current();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onCloseRef.current();
      anchor.current?.focus();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open, anchor]);

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          ref={ref}
          role="dialog"
          aria-label={label}
          style={{ position: "fixed", top: pos.top, left: pos.left, width: pos.width }}
          className={cx(
            "z-[600] flex min-w-[220px] flex-col rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] p-1.5 shadow-[var(--shadow-popup)]",
            className,
          )}
          initial={{ opacity: 0, y: pos.flipped ? 4 : -4, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: pos.flipped ? 4 : -4, scale: 0.97 }}
          transition={{ duration: 0.12, ease: "easeOut" }}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
