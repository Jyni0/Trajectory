import { create } from "zustand";
import { api, errorMessage, isTauri } from "@/lib/api";

/**
 * Updates from the public GitHub releases (like Gravitation): the latest
 * release is checked a few seconds after launch and every few hours; a newer
 * one shows "Update" on Settings in the sidebar and in Settings → About,
 * where it is downloaded, installed and the app restarted.
 */
export type UpdateStatus = "idle" | "checking" | "uptodate" | "available" | "downloading" | "installing" | "error";

/** A newer GitHub release, as `check_update` reports it. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
  pageUrl: string;
  /** Installer for this system; null when the release has none. */
  assetName: string | null;
}

interface UpdateState {
  status: UpdateStatus;
  /** The newer release, once found. */
  release: UpdateInfo | null;
  /** 0–1 while downloading, null when the size is unknown. */
  progress: number | null;
  error: string | null;
  check(): Promise<void>;
  install(): Promise<void>;
  /** The release page on GitHub (only needed when it has no build for this system). */
  openRelease(): Promise<void>;
}

async function invoke<T>(cmd: string): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd);
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: "idle",
  release: null,
  progress: null,
  error: null,

  async check() {
    const { status } = get();
    if (!isTauri() || status === "checking" || status === "downloading" || status === "installing") return;
    set({ status: "checking", error: null });
    try {
      const release = await invoke<UpdateInfo | null>("check_update");
      set({ status: release ? "available" : "uptodate", release });
    } catch (e) {
      // A release found earlier stays offered.
      set({ status: get().release ? "available" : "error", error: errorMessage(e) });
    }
  },

  async install() {
    const { status, release } = get();
    if (!release?.assetName || (status !== "available" && status !== "error")) return;
    set({ status: "downloading", progress: null, error: null });
    const { listen } = await import("@tauri-apps/api/event");
    const off = await listen<{ downloaded: number; total: number | null }>("update:progress", ({ payload }) => {
      const progress = payload.total ? Math.min(1, payload.downloaded / payload.total) : null;
      set({ progress, status: progress === 1 ? "installing" : "downloading" });
    });
    try {
      // On success the app quits and the new version starts.
      await invoke("install_update");
      set({ status: "installing" });
    } catch (e) {
      set({ status: "error", error: errorMessage(e) });
    } finally {
      off();
    }
  },

  async openRelease() {
    const url = get().release?.pageUrl ?? "https://github.com/Jyni0/Trajectory/releases/latest";
    await api.openExternal(url).catch((e) => set({ error: errorMessage(e) }));
  },
}));

/** A newer release is known (the sidebar shows "Update"). */
export const selectHasUpdate = (s: UpdateState) => !!s.release && s.status !== "uptodate";

/** First check a few seconds after launch, then every few hours. */
const FIRST_CHECK_MS = 5_000;
const CHECK_EVERY_MS = 6 * 3600_000;

export function startUpdateChecks(): () => void {
  if (!isTauri()) return () => {};
  const run = () => {
    const { status } = useUpdateStore.getState();
    if (status !== "downloading" && status !== "installing") void useUpdateStore.getState().check();
  };
  const first = setTimeout(run, FIRST_CHECK_MS);
  const every = setInterval(run, CHECK_EVERY_MS);
  return () => {
    clearTimeout(first);
    clearInterval(every);
  };
}
