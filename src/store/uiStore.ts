import { create } from "zustand";
import type { ComposerDraft } from "@/lib/compose";
import { applyTheme, isTheme, type Theme } from "@/lib/themes";

export type SettingsTab = "appearance" | "accounts" | "add" | "oauth" | "general" | "about";
export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

export interface Preferences {
  loadRemoteImages: boolean;
  notifications: boolean;
  theme: Theme;
  sidebarHidden: boolean;
  /** Show BIMI brand logos of authenticated senders (DNS + HTTPS lookups). */
  senderLogos: boolean;
}

interface UiState {
  settingsOpen: boolean;
  settingsTab: SettingsTab;
  composer: ComposerDraft | null;
  toasts: Toast[];
  prefs: Preferences;
  openSettings(tab?: SettingsTab): void;
  closeSettings(): void;
  /** Opens a draft; one already open is first saved (if changed) and closed. */
  openComposer(draft: ComposerDraft): void;
  closeComposer(): void;
  /** Leaves the open composer: changes go to Drafts, an untouched one just
   *  closes. False when the draft could not be saved (it stays open). */
  leaveComposer(): Promise<boolean>;
  pushToast(message: string, kind?: ToastKind): number;
  dismissToast(id: number): void;
  setPref<K extends keyof Preferences>(key: K, value: Preferences[K]): void;
}

const PREFS_KEY = "trajectory.prefs";
/** Where prefs lived before the app was renamed; read once, then moved. */
const LEGACY_PREFS_KEY = "singularity.prefs";
const DEFAULT_PREFS: Preferences = { loadRemoteImages: false, notifications: true, theme: "dark", sidebarHidden: false, senderLogos: true };

function loadPrefs(): Preferences {
  try {
    let raw = localStorage.getItem(PREFS_KEY);
    if (raw === null && (raw = localStorage.getItem(LEGACY_PREFS_KEY)) !== null) {
      localStorage.setItem(PREFS_KEY, raw);
      localStorage.removeItem(LEGACY_PREFS_KEY);
    }
    const p = { ...DEFAULT_PREFS, ...JSON.parse(raw ?? "{}") } as Preferences;
    if (!isTheme(p.theme)) p.theme = DEFAULT_PREFS.theme;
    return p;
  } catch {
    return DEFAULT_PREFS;
  }
}

const initialPrefs = loadPrefs();
if (typeof document !== "undefined") applyTheme(initialPrefs.theme);

let toastId = 0;

/** Set by the mounted Composer: saves it to Drafts when changed, then closes it. */
let composerLeave: (() => Promise<boolean>) | null = null;
export function setComposerLeave(fn: (() => Promise<boolean>) | null) {
  composerLeave = fn;
}

export const useUiStore = create<UiState>((set, get) => ({
  settingsOpen: false,
  settingsTab: "accounts",
  composer: null,
  toasts: [],
  prefs: initialPrefs,

  openSettings: (tab = "accounts") => set({ settingsOpen: true, settingsTab: tab }),
  closeSettings: () => set({ settingsOpen: false }),
  openComposer(draft) {
    if (!get().composer) return set({ composer: draft });
    void get()
      .leaveComposer()
      .then((left) => left && set({ composer: draft }));
  },
  closeComposer: () => set({ composer: null }),
  async leaveComposer() {
    if (!get().composer) return true;
    if (composerLeave && !(await composerLeave())) return false;
    set({ composer: null });
    return true;
  },

  pushToast(message, kind = "info") {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, kind, message }] }));
    setTimeout(() => get().dismissToast(id), kind === "error" ? 6000 : 3500);
    return id;
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setPref(key, value) {
    const prefs = { ...get().prefs, [key]: value };
    // Before React re-renders, so everything drawn next uses the new colours.
    if (key === "theme") applyTheme(value as Theme);
    set({ prefs });
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
    } catch {
      /* storage unavailable */
    }
  },
}));
