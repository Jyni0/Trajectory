import { create } from "zustand";
import { errorMessage, isTauri } from "@/lib/api";

export type UpdateStatus = "idle" | "checking" | "uptodate" | "available" | "downloading" | "error";

/** A newer GitHub release, as `check_update` reports it. */
interface UpdateInfo {
  version: string;
  notes: string | null;
  pageUrl: string;
  /** Installer the app applies itself; null = only the release page. */
  assetName: string | null;
}

interface UpdateState {
  status: UpdateStatus;
  /** Version offered by the latest GitHub release. */
  version: string | null;
  /** 0–1 while downloading, null when the size is unknown. */
  progress: number | null;
  error: string | null;
  /** The banner was closed with "Later" (the About page still offers it). */
  dismissed: boolean;
  /** No installer for this platform: "Update" opens the release page. */
  manual: boolean;
  check(): Promise<void>;
  install(): Promise<void>;
  dismiss(): void;
}

async function invoke<T>(cmd: string): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd);
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: "idle",
  version: null,
  progress: null,
  error: null,
  dismissed: false,
  manual: false,

  async check() {
    const { status } = get();
    if (!isTauri() || status === "checking" || status === "downloading") return;
    set({ status: "checking", error: null });
    try {
      const info = await invoke<UpdateInfo | null>("check_update");
      if (info) set({ status: "available", version: info.version, manual: !info.assetName });
      else set({ status: "uptodate", version: null });
    } catch (e) {
      set({ status: "error", error: errorMessage(e) });
    }
  },

  async install() {
    const { status, manual } = get();
    if (status !== "available") return;
    // Nothing to apply here (e.g. a .deb install): the release page opens.
    if (manual) {
      await invoke("install_update").catch((e) => set({ status: "error", error: errorMessage(e) }));
      return;
    }
    set({ status: "downloading", progress: null, error: null, dismissed: false });
    const { listen } = await import("@tauri-apps/api/event");
    const off = await listen<{ downloaded: number; total: number | null }>("update:progress", ({ payload }) =>
      set({ progress: payload.total ? Math.min(1, payload.downloaded / payload.total) : null }),
    );
    try {
      // On success the app quits and the new version starts.
      await invoke("install_update");
    } catch (e) {
      set({ status: "error", error: errorMessage(e) });
    } finally {
      off();
    }
  },

  dismiss: () => set({ dismissed: true }),
}));

/** First check shortly after start, then every few hours. */
const FIRST_CHECK_MS = 10_000;
const CHECK_EVERY_MS = 6 * 3600_000;

export function startUpdateChecks(): () => void {
  if (!isTauri()) return () => {};
  const run = () => {
    const { status } = useUpdateStore.getState();
    if (status !== "available" && status !== "downloading") void useUpdateStore.getState().check();
  };
  const first = setTimeout(run, FIRST_CHECK_MS);
  const every = setInterval(run, CHECK_EVERY_MS);
  return () => {
    clearTimeout(first);
    clearInterval(every);
  };
}
