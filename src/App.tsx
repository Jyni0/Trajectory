import { useEffect } from "react";
import { motion } from "motion/react";
import { Mail, Plus } from "lucide-react";
import { Sidebar } from "@/components/layout/Sidebar";
import { TitleBar } from "@/components/layout/TitleBar";
import { Toaster } from "@/components/layout/Toaster";
import { EmailList } from "@/components/mail/EmailList";
import { MailReader } from "@/components/mail/MailReader";
import { Composer, composerKey } from "@/components/mail/Composer";
import { SettingsModal } from "@/components/settings/SettingsModal";
import { Button, Spinner, cx } from "@/components";
import { useMailEvents } from "@/hooks/useMailEvents";
import { useHotkeys } from "@/hooks/useHotkeys";
import { emptyDraft } from "@/lib/compose";
import { applyTheme } from "@/lib/themes";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";

/** Bar widths of the ghost rows (sender, subject) — uneven, like real mail. */
const GHOST_ROWS: [number, number][] = [
  [26, 58],
  [34, 46],
  [22, 64],
  [30, 52],
];

/**
 * No account yet: what the inbox will look like — ghost rows fading into the
 * page — with how to fill it on top. Same empty state as Gravitation.
 */
function Onboarding() {
  const openSettings = useUiStore((s) => s.openSettings);
  return (
    <motion.div
      className="mx-auto flex w-full max-w-[560px] flex-col items-center pt-[14vh]"
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: "easeOut" }}
    >
      <h1 className="text-[14px] font-medium text-[var(--text-main)]">No accounts yet</h1>
      <div className="mt-1 max-w-[380px] text-center text-[12.5px] leading-relaxed text-[var(--text-dim)]">
        Connect Gmail, Outlook or any IMAP / POP3 mailbox. Passwords stay in the system keychain.
      </div>
      <Button variant="primary" className="mt-4" icon={<Plus size={14} />} onClick={() => openSettings("add")}>
        Add your first account
      </Button>
      <div
        aria-hidden
        className="mt-8 flex w-full select-none flex-col gap-0.5"
        style={{ maskImage: "linear-gradient(to bottom, black 10%, transparent 95%)" }}
      >
        {GHOST_ROWS.map(([from, subject], i) => (
          <div key={i} className="flex items-center gap-3 px-3 py-2.5">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[var(--bg-input)] text-[var(--text-dim)]">
              <Mail size={13} strokeWidth={1.6} />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-1.5">
              <span className="h-2.5 rounded-full bg-[var(--bg-input)]" style={{ width: from + "%" }} />
              <span className="h-2 rounded-full bg-[var(--bg-input)] opacity-70" style={{ width: subject + "%" }} />
            </span>
          </div>
        ))}
      </div>
    </motion.div>
  );
}

export default function App() {
  const initialized = useMailStore((s) => s.initialized);
  const hasAccounts = useMailStore((s) => s.accounts.length > 0);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const error = useMailStore((s) => s.error);
  const pushToast = useUiStore((s) => s.pushToast);
  const theme = useUiStore((s) => s.prefs.theme);
  const sidebarHidden = useUiStore((s) => s.prefs.sidebarHidden);
  const composer = useUiStore((s) => s.composer);
  const { openSettings, openComposer, setPref } = useUiStore.getState();

  useMailEvents();
  useHotkeys();

  useEffect(() => {
    void useMailStore.getState().init();
  }, []);

  useEffect(() => applyTheme(theme), [theme]);

  useEffect(() => {
    if (error) {
      pushToast(error, "error");
      useMailStore.setState({ error: null });
    }
  }, [error, pushToast]);

  const showSidebar = initialized && hasAccounts && !sidebarHidden;

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <TitleBar
        canCompose={!!activeAccountId}
        onNewMessage={() => activeAccountId && openComposer(emptyDraft(activeAccountId))}
        onOpenSettings={() => openSettings("appearance")}
        onToggleSidebar={() => setPref("sidebarHidden", !sidebarHidden)}
        sidebarHidden={sidebarHidden}
      />
      {/* overflow-hidden: the row never scrolls sideways. */}
      <div className="flex min-h-0 flex-1 overflow-hidden bg-[var(--bg-sidebar)]">
        {showSidebar && <Sidebar />}
        <main
          className={cx(
            "flex min-w-0 flex-1 overflow-hidden bg-[var(--bg-app)]",
            // The page sits in the corner between the sidebar and the title
            // bar like a sheet: rounded top-left, a hairline along both edges.
            showSidebar && "rounded-tl-2xl border-l border-t border-[var(--border-soft)]",
          )}
        >
          {!initialized ? (
            <div className="flex flex-1 items-center justify-center text-[var(--text-dim)]">
              <Spinner size={18} />
            </div>
          ) : hasAccounts ? (
            <>
              <EmailList />
              {/* Writing replaces the reading pane. */}
              {composer ? <Composer key={composerKey(composer)} initial={composer} /> : <MailReader />}
            </>
          ) : (
            <Onboarding />
          )}
        </main>
      </div>
      <SettingsModal />
      <Toaster />
    </div>
  );
}
