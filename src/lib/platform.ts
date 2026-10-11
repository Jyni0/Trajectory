/** Running on macOS (the window gets native traffic-light buttons there). */
export const isMac: boolean = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);

/** "Ctrl+N" as this platform writes it: "⌘N" on macOS. */
export function shortcutLabel(combo: string): string {
  return isMac ? combo.replace(/^Ctrl\+/, "⌘") : combo;
}
