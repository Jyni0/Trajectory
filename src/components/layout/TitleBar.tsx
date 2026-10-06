import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Copy, Minus, Square, X } from "lucide-react";
import { isTauri } from "@/lib/api";
import { shortcutKey } from "@/lib/keys";

interface MenuItem {
  label: string;
  /** Shown on the right; the same combo is bound globally below. */
  shortcut?: string;
  disabled?: boolean;
  run: () => void;
}
type MenuEntry = MenuItem | "separator";

async function win() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

/**
 * The window's own title bar (the native frame is off): app name, App / View
 * menus with their shortcuts, and the window buttons. Closing hides the
 * window to the tray (background sync keeps running); App → Quit exits.
 */
export function TitleBar({
  onNewMessage,
  onOpenSettings,
  onToggleSidebar,
  sidebarHidden,
  canCompose,
}: {
  onNewMessage: () => void;
  onOpenSettings: () => void;
  onToggleSidebar: () => void;
  sidebarHidden: boolean;
  canCompose: boolean;
}) {
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [maximized, setMaximized] = useState(false);
  const barRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let off: (() => void) | undefined;
    void win().then(async (w) => {
      setMaximized(await w.isMaximized());
      off = await w.onResized(() => void w.isMaximized().then(setMaximized));
    });
    return () => off?.();
  }, []);

  useEffect(() => {
    if (!openMenu) return;
    const close = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) setOpenMenu(null);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpenMenu(null);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [openMenu]);

  const minimize = () => isTauri() && void win().then((w) => w.minimize());
  const toggleMax = () => isTauri() && void win().then((w) => w.toggleMaximize());
  const close = () => isTauri() && void win().then((w) => w.close());
  const quit = async () => {
    if (!isTauri()) return;
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("quit_app");
  };
  const toggleFullscreen = async () => {
    if (!isTauri()) {
      if (document.fullscreenElement) void document.exitFullscreen();
      else void document.documentElement.requestFullscreen();
      return;
    }
    const w = await win();
    await w.setFullscreen(!(await w.isFullscreen()));
  };

  const MENUS: Record<string, MenuEntry[]> = {
    App: [
      { label: "New Message", shortcut: "Ctrl+N", disabled: !canCompose, run: onNewMessage },
      "separator",
      { label: "Settings", shortcut: "Ctrl+,", run: onOpenSettings },
      "separator",
      { label: "Quit Trajectory", shortcut: "Ctrl+Q", run: () => void quit() },
    ],
    View: [
      { label: sidebarHidden ? "Show Sidebar" : "Hide Sidebar", shortcut: "Ctrl+B", run: onToggleSidebar },
      { label: "Toggle Fullscreen", shortcut: "F11", run: () => void toggleFullscreen() },
      "separator",
      { label: "Reload", shortcut: "Ctrl+R", run: () => location.reload() },
    ],
  };

  // Keyboard shortcuts of the menu entries (physical keys, any layout).
  const menusRef = useRef(MENUS);
  menusRef.current = MENUS;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.defaultPrevented) return;
      let combo = "";
      if (e.key === "F11") combo = "F11";
      else if ((e.ctrlKey || e.metaKey) && !e.shiftKey) combo = "Ctrl+" + shortcutKey(e);
      if (!combo) return;
      for (const entries of Object.values(menusRef.current)) {
        for (const item of entries) {
          if (item !== "separator" && item.shortcut === combo) {
            e.preventDefault();
            setOpenMenu(null);
            if (!item.disabled) item.run();
            return;
          }
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const WIN_BTN =
    "flex w-[54px] items-center justify-center text-[var(--text-muted)] transition-colors hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]";

  return (
    <div ref={barRef} data-tauri-drag-region className="flex h-[34px] shrink-0 select-none items-center bg-[var(--bg-titlebar)]">
      {/* Left: app menu */}
      <div className="flex h-full items-center gap-1 pl-2">
        <span className="mr-0.5 bg-[var(--text-muted)] bg-clip-text px-2 text-[13px] font-semibold tracking-wide text-transparent">
          Trajectory
        </span>
        {Object.keys(MENUS).map((m) => (
          <div key={m} className="relative flex h-full items-center">
            <button
              type="button"
              className={`mr-0.5 rounded-md px-2.5 py-1 text-[13px] transition-colors ${
                openMenu === m
                  ? "bg-[var(--hover-bg)] text-[var(--text-main)]"
                  : "text-[var(--text-muted)] hover:bg-[var(--hover-bg)] hover:text-[var(--text-main)]"
              }`}
              onMouseDown={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setOpenMenu(openMenu === m ? null : m);
              }}
              onMouseEnter={() => openMenu && setOpenMenu(m)}
            >
              {m}
            </button>
            <AnimatePresence>
              {openMenu === m && (
                <motion.div
                  role="menu"
                  className="absolute left-0 top-[calc(100%+4px)] z-[300] flex min-w-[230px] flex-col gap-0.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 shadow-[var(--shadow-popup)]"
                  initial={{ opacity: 0, y: -4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.12 }}
                >
                  {MENUS[m].map((item, i) =>
                    item === "separator" ? (
                      <div key={`sep-${i}`} className="mx-1.5 my-0.5 h-px bg-[var(--border)]" />
                    ) : (
                      <button
                        key={item.label}
                        type="button"
                        role="menuitem"
                        disabled={item.disabled}
                        className="flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left text-[13px] text-[var(--text-main)] transition-colors hover:bg-[var(--hover-bg)] disabled:opacity-40 disabled:hover:bg-transparent"
                        onClick={() => {
                          setOpenMenu(null);
                          item.run();
                        }}
                      >
                        <span className="flex-1">{item.label}</span>
                        {item.shortcut && <span className="pl-4 text-[11px] text-[var(--text-dim)]">{item.shortcut}</span>}
                      </button>
                    ),
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        ))}
      </div>

      {/* Center: drag region */}
      <div className="h-full flex-1" data-tauri-drag-region />

      {/* Right: window controls — 54px wide, full height */}
      <div className="flex h-full items-stretch">
        <button type="button" className={WIN_BTN} onClick={minimize} title="Minimize">
          <Minus size={15} strokeWidth={1} />
        </button>
        <button type="button" className={WIN_BTN} onClick={toggleMax} title={maximized ? "Restore" : "Maximize"}>
          {maximized ? <Copy size={13} strokeWidth={1} /> : <Square size={13} strokeWidth={1} />}
        </button>
        <button
          type="button"
          className="flex w-[54px] items-center justify-center text-[var(--text-muted)] transition-colors hover:bg-[#E81123] hover:text-white"
          onClick={close}
          title="Close to tray"
        >
          <X size={15} strokeWidth={1.2} />
        </button>
      </div>
    </div>
  );
}
