import { AnimatePresence, motion } from "motion/react";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { useUiStore } from "@/store/uiStore";
import { cx } from "@/components";

const ICONS = { info: Info, success: CheckCircle2, error: AlertCircle };

export function Toaster() {
  const toasts = useUiStore((s) => s.toasts);
  const dismiss = useUiStore((s) => s.dismissToast);
  return (
    <div className="pointer-events-none fixed bottom-5 left-1/2 z-[500] flex -translate-x-1/2 flex-col items-center gap-2" aria-live="polite">
      <AnimatePresence initial={false}>
        {toasts.map((t) => {
          const Icon = ICONS[t.kind];
          return (
            <motion.div
              key={t.id}
              layout
              role={t.kind === "error" ? "alert" : "status"}
              initial={{ opacity: 0, y: 12, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 6, scale: 0.98 }}
              transition={{ type: "spring", stiffness: 500, damping: 34 }}
              className="pointer-events-auto flex h-10 max-w-md items-center gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] pl-3 pr-1.5 text-[13px] text-[var(--text-main)] shadow-[var(--shadow-popup)]"
            >
              <Icon size={15} className={cx(t.kind === "success" && "text-[var(--diff-add)]", t.kind === "error" && "text-[var(--diff-del)]", t.kind === "info" && "text-[var(--accent)]")} />
              <span className="truncate">{t.message}</span>
              <button type="button" aria-label="Dismiss" onClick={() => dismiss(t.id)} className="rounded-lg p-1 text-[var(--text-dim)] hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]">
                <X size={13} />
              </button>
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
}
