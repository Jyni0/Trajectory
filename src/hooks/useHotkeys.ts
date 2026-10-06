import { useEffect } from "react";
import { emptyDraft, replyDraft } from "@/lib/compose";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";

const isTyping = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

/** Asks the reader to open its tag picker (the `t` shortcut). */
export const TAG_PICKER_EVENT = "trajectory:tag-picker";

/** Gmail-style single-key shortcuts: c, r, j/k, e, #, s, u, /, t, 1–9 (toggle tag N). */
export function useHotkeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target)) return;
      const ui = useUiStore.getState();
      if (ui.composer || ui.settingsOpen) return;
      const mail = useMailStore.getState();
      const email = mail.activeEmail;
      const account = mail.activeAccount();

      switch (e.key) {
        case "c":
          if (account) ui.openComposer(emptyDraft(account.id));
          break;
        case "r":
          if (email && account) ui.openComposer(replyDraft(email, account.email));
          break;
        case "j":
        case "ArrowDown":
          void mail.selectNext(1);
          break;
        case "k":
        case "ArrowUp":
          void mail.selectNext(-1);
          break;
        case "e":
          if (mail.selectedEmailId) void mail.archive(mail.selectedEmailId);
          break;
        case "#":
        case "Delete":
          if (mail.selectedEmailId) void mail.remove(mail.selectedEmailId);
          break;
        case "s":
          if (mail.selectedEmailId) void mail.toggleFlag(mail.selectedEmailId);
          break;
        case "u":
          if (email) void mail.markRead(email.id, !email.isRead);
          break;
        case "/":
          document.getElementById("mail-search")?.focus();
          break;
        case "t":
          if (!email) return;
          window.dispatchEvent(new Event(TAG_PICKER_EVENT));
          break;
        default: {
          const n = Number(e.key);
          const tag = Number.isInteger(n) && n >= 1 ? mail.tags[n - 1] : undefined;
          if (!tag || !mail.selectedEmailId) return;
          void mail.tagEmail(mail.selectedEmailId, tag.id);
        }
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
