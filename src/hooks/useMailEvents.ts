import { useEffect } from "react";
import { api, type Unlisten } from "@/lib/api";
import { displayName } from "@/lib/format";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";

/** Bridges Rust events (`mail:new`, `mail:changed`, `sync:*`) into the stores. */
export function useMailEvents() {
  useEffect(() => {
    const subs: Promise<Unlisten>[] = [];
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;

    // Coalesce bursts of change events (e.g. many folders synced at once).
    const refresh = (accountId: string) => {
      if (useMailStore.getState().activeAccountId !== accountId) return;
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        const s = useMailStore.getState();
        void s.refreshFolders();
        void s.loadEmails();
        void s.loadTags();
      }, 150);
    };

    subs.push(api.on("mail:changed", (e) => refresh(e.accountId)));
    subs.push(
      api.on("mail:new", (e) => {
        refresh(e.accountId);
        const { prefs, pushToast } = useUiStore.getState();
        const first = e.latest[0];
        const text = e.count === 1 && first ? `${displayName(first.from)}: ${first.subject}` : `${e.count} new messages`;
        if (prefs.notifications && typeof Notification !== "undefined" && document.hidden) {
          if (Notification.permission === "granted") new Notification("New mail", { body: text });
          else if (Notification.permission === "default") void Notification.requestPermission();
        } else {
          pushToast(text, "info");
        }
      }),
    );
    subs.push(api.on("sync:status", (e) => useMailStore.getState().setSyncState(e.accountId, e.state)));
    subs.push(
      api.on("sync:error", (e) => {
        useUiStore.getState().pushToast(e.error, "error");
      }),
    );

    return () => {
      clearTimeout(refreshTimer);
      subs.forEach((p) => void p.then((un) => un()));
    };
  }, []);
}
