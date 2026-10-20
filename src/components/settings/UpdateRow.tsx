import { Download, ExternalLink, RefreshCw } from "lucide-react";
import { isTauri } from "@/lib/api";
import { useUpdateStore } from "@/store/updateStore";
import { Alert, Button, SettingRow, Spinner } from "@/components";

const APP_VERSION: string = import.meta.env.VITE_APP_VERSION ?? "0.0.0";

/** Settings → About: a newer GitHub release, installed in place with one click. */
export function UpdateRow() {
  const status = useUpdateStore((s) => s.status);
  const release = useUpdateStore((s) => s.release);
  const progress = useUpdateStore((s) => s.progress);
  const error = useUpdateStore((s) => s.error);
  const { check, install, openRelease } = useUpdateStore.getState();
  const offered = !!release && status !== "uptodate";
  const busy = status === "downloading" || status === "installing";
  const installable = !!release?.assetName;

  const hint = !isTauri()
    ? "Available in the desktop app."
    : status === "checking"
      ? "Checking GitHub releases…"
      : status === "downloading"
        ? `Downloading ${release?.version}${progress !== null ? ` — ${Math.round(progress * 100)}%` : "…"}`
        : status === "installing"
          ? `Installing ${release?.version} — Trajectory restarts by itself.`
          : offered && installable
            ? `Version ${release.version} is out — you have ${APP_VERSION}.`
            : offered
              ? `Version ${release.version} is out, but the release has no build for this system.`
              : status === "uptodate"
                ? `You have the latest version (${APP_VERSION}).`
                : "Checks the GitHub releases for a newer version.";

  return (
    <>
      <SettingRow title="Updates" hint={hint}>
        {offered && installable ? (
          <Button
            variant="primary"
            data-testid="install-update"
            icon={busy ? <Spinner size={13} /> : <Download size={13} />}
            disabled={busy}
            onClick={() => void install()}
          >
            {busy ? "Updating…" : `Update to ${release.version}`}
          </Button>
        ) : offered ? (
          <Button icon={<ExternalLink size={13} />} onClick={() => void openRelease()}>
            Open release {release.version}
          </Button>
        ) : (
          <Button
            icon={status === "checking" ? <Spinner size={13} /> : <RefreshCw size={13} />}
            disabled={!isTauri() || status === "checking"}
            onClick={() => void check()}
          >
            Check for updates
          </Button>
        )}
      </SettingRow>
      {busy && (
        <div className="h-1 overflow-hidden rounded-full bg-[var(--bg-input)]">
          <div
            className={progress === null ? "h-full w-1/3 animate-pulse bg-[var(--accent)]" : "h-full bg-[var(--accent)] transition-[width]"}
            style={progress === null ? undefined : { width: `${Math.round(progress * 100)}%` }}
          />
        </div>
      )}
      {offered && release.notes && (
        <div className="max-h-40 overflow-auto whitespace-pre-wrap rounded-xl bg-[var(--bg-input)] px-3 py-2 text-[12px] leading-relaxed text-[var(--text-muted)]">
          {release.notes}
        </div>
      )}
      {error && !busy && <Alert>Update failed: {error}</Alert>}
    </>
  );
}
