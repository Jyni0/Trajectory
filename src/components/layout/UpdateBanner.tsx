import { AnimatePresence, motion } from "motion/react";
import { Download, X } from "lucide-react";
import { Button } from "@/components";
import { useUpdateStore } from "@/store/updateStore";

/** Bottom-right card offering a newer release; installs and restarts. */
export function UpdateBanner() {
  const status = useUpdateStore((s) => s.status);
  const version = useUpdateStore((s) => s.version);
  const progress = useUpdateStore((s) => s.progress);
  const dismissed = useUpdateStore((s) => s.dismissed);
  const manual = useUpdateStore((s) => s.manual);
  const { install, dismiss } = useUpdateStore.getState();
  const show = !dismissed && (status === "available" || status === "downloading");

  return (
    <AnimatePresence>
      {show && (
        <motion.div
          role="status"
          data-testid="update-banner"
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 500, damping: 34 }}
          className="fixed bottom-5 right-5 z-[450] flex w-[300px] flex-col gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 text-[13px] text-[var(--text-main)] shadow-[var(--shadow-popup)]"
        >
          <div className="flex items-start gap-2.5">
            <Download size={15} className="mt-0.5 shrink-0 text-[var(--accent)]" />
            <div className="flex min-w-0 flex-1 flex-col leading-snug">
              <span className="font-medium">Trajectory {version} is available</span>
              <span className="text-[12px] text-[var(--text-dim)]">
                {status === "downloading"
                  ? "Downloading — the app restarts when it's done."
                  : manual
                    ? "Download it from the release page."
                    : "Install it now; the app restarts."}
              </span>
            </div>
            {status === "available" && (
              <button
                type="button"
                aria-label="Later"
                onClick={dismiss}
                className="rounded-lg p-1 text-[var(--text-dim)] hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]"
              >
                <X size={13} />
              </button>
            )}
          </div>
          {status === "downloading" ? (
            <div className="h-1 overflow-hidden rounded-full bg-[var(--bg-input)]">
              <div
                className={progress === null ? "h-full w-1/3 animate-pulse bg-[var(--accent)]" : "h-full bg-[var(--accent)] transition-[width]"}
                style={progress === null ? undefined : { width: `${Math.round(progress * 100)}%` }}
              />
            </div>
          ) : (
            <div className="flex justify-end gap-2">
              <Button size="sm" onClick={dismiss}>
                Later
              </Button>
              <Button size="sm" variant="primary" onClick={() => void install()}>
                {manual ? "Open release page" : "Update and restart"}
              </Button>
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
