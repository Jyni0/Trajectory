import { create } from "zustand";
import { errorMessage, isTauri } from "@/lib/api";

export type UpdateStatus = "idle" | "checking" | "uptodate" | "available" | "downloading" | "error";

/** What the updater plugin returns for a release newer than this build. */
interface Pending {
  version: string;
  install(onProgress: (done: number, total: number | null) => void): Promise<void>;
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
  check(): Promise<void>;
  install(): Promise<void>;
  dismiss(): void;
}

let pending: Pending | null = null;

/** Asks GitHub Releases (`latest.json`) for a newer, signed build. */
async function findUpdate(): Promise<Pending | null> {
  if (!isTauri()) return null;
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    async install(onProgress) {
      let total: number | null = null;
      let done = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? null;
        else if (e.event === "Progress") onProgress((done += e.data.chunkLength), total);
      });
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    },
  };
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: "idle",
  version: null,
  progress: null,
  error: null,
  dismissed: false,

  async check() {
    const { status } = get();
    if (status === "checking" || status === "downloading") return;
    set({ status: "checking", error: null });
    try {
      pending = await findUpdate();
      if (pending) set({ status: "available", version: pending.version });
      else set({ status: "uptodate", version: null });
    } catch (e) {
      set({ status: "error", error: errorMessage(e) });
    }
  },

  async install() {
    if (!pending || get().status === "downloading") return;
    set({ status: "downloading", progress: null, error: null, dismissed: false });
    try {
      await pending.install((done, total) => set({ progress: total ? Math.min(1, done / total) : null }));
    } catch (e) {
      set({ status: "error", error: errorMessage(e) });
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
