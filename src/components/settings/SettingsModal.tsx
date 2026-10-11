import { useState } from "react";
import { Info, KeyRound, Palette, Plus, RefreshCw, SlidersHorizontal, Trash2, Users, X, type LucideIcon } from "lucide-react";
import { useMailStore } from "@/store/mailStore";
import { useUiStore, type SettingsTab } from "@/store/uiStore";
import { useUpdateStore } from "@/store/updateStore";
import { errorMessage, isTauri } from "@/lib/api";
import { THEME_LIST, type Theme } from "@/lib/themes";
import {
  Button,
  Combobox,
  ConfirmBar,
  Dialog,
  IconButton,
  NavItem,
  Pill,
  ScrollArea,
  Sep,
  SettingRow,
  SettingsCard,
  Switch,
} from "@/components";
import { AddAccountPanel } from "./AddAccountPanel";
import { OAuthAppsPanel } from "./OAuthAppsPanel";
import { SenderAvatar, VerifiedMark, domainOf } from "@/components/mail/SenderAvatar";

const APP_VERSION: string = import.meta.env.VITE_APP_VERSION ?? "0.0.0";

const KIND_LABEL = { gmail: "Google · OAuth", outlook: "Microsoft · OAuth", imap: "IMAP + SMTP", pop3: "POP3 + SMTP" } as const;

function AppearancePanel() {
  const theme = useUiStore((s) => s.prefs.theme);
  const setPref = useUiStore((s) => s.setPref);
  return (
    <SettingsCard>
      <SettingRow title="Theme" hint="Application color scheme — searchable">
        <div className="w-[240px]">
          <Combobox
            value={theme}
            onChange={(v) => setPref("theme", v as Theme)}
            placeholder="Search themes…"
            emptyText="No theme matches"
            options={THEME_LIST.map((t) => ({ value: t.id, label: t.label, hint: t.kind, swatch: t.swatch }))}
          />
        </div>
      </SettingRow>
    </SettingsCard>
  );
}

function AccountsPanel() {
  const accounts = useMailStore((s) => s.accounts);
  const syncState = useMailStore((s) => s.syncState);
  const { removeAccount, sync } = useMailStore.getState();
  const { openSettings, pushToast } = useUiStore.getState();
  const [confirm, setConfirm] = useState<string | null>(null);

  return (
    <div className="flex flex-col gap-4">
      <SettingsCard>
        <SettingRow title="Mail accounts" hint="Passwords and tokens are kept in the system keychain, never on disk.">
          <Button variant="primary" icon={<Plus size={13} />} onClick={() => openSettings("add")}>
            Add account
          </Button>
        </SettingRow>
      </SettingsCard>

      {accounts.length > 0 && (
        <SettingsCard className="gap-1 px-2 py-2">
          {accounts.map((a, i) => (
            <div key={a.id} className="flex flex-col">
              {i > 0 && <div className="mx-2 my-1 h-px bg-[var(--border-soft)]" />}
              {confirm === a.id ? (
                <ConfirmBar
                  message={
                    <>
                      Remove <b>{a.email}</b> from this device?
                    </>
                  }
                  confirmLabel="Remove"
                  onCancel={() => setConfirm(null)}
                  onConfirm={async () => {
                    try {
                      await removeAccount(a.id);
                      pushToast(`${a.email} removed`, "info");
                    } catch (e) {
                      pushToast(errorMessage(e), "error");
                    }
                    setConfirm(null);
                  }}
                />
              ) : (
                <div className="group flex items-center gap-3 rounded-xl px-2 py-1.5">
                  <SenderAvatar from={{ name: a.displayName, address: a.email }} logoDomain={domainOf(a.email)} color={a.color} size={30} />
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="flex min-w-0 items-center gap-1">
                      <VerifiedMark logoDomain={domainOf(a.email)} size={13} />
                      <span className="truncate text-[13px] text-[var(--text-main)]">{a.displayName || a.email}</span>
                    </span>
                    <span className="truncate text-[12px] text-[var(--text-dim)]">{a.email}</span>
                  </div>
                  {syncState[a.id] === "error" && <Pill tone="danger">Sync error</Pill>}
                  <Pill>{KIND_LABEL[a.kind]}</Pill>
                  <IconButton label={`Sync ${a.email}`} onClick={() => void sync(a.id)}>
                    <RefreshCw size={13} />
                  </IconButton>
                  <IconButton label={`Remove ${a.email}`} tone="danger" onClick={() => setConfirm(a.id)}>
                    <Trash2 size={13} />
                  </IconButton>
                </div>
              )}
            </div>
          ))}
        </SettingsCard>
      )}
    </div>
  );
}

function GeneralPanel() {
  const prefs = useUiStore((s) => s.prefs);
  const setPref = useUiStore((s) => s.setPref);
  return (
    <SettingsCard>
      <SettingRow title="Load remote images" hint="Remote content can tell the sender when you open a message.">
        <Switch on={prefs.loadRemoteImages} onChange={(v) => setPref("loadRemoteImages", v)} ariaLabel="Load remote images" />
      </SettingRow>
      <Sep />
      <SettingRow title="Desktop notifications" hint="Notify when new mail arrives in an inbox.">
        <Switch on={prefs.notifications} onChange={(v) => setPref("notifications", v)} ariaLabel="Desktop notifications" />
      </SettingRow>
      <Sep />
      <SettingRow
        title="Sender logos (BIMI)"
        hint="Brand logos of senders that passed DMARC, looked up in DNS and fetched from the brand once a week. A check mark means the logo is backed by a Verified Mark Certificate."
      >
        <Switch on={prefs.senderLogos} onChange={(v) => setPref("senderLogos", v)} ariaLabel="Sender logos" />
      </SettingRow>
    </SettingsCard>
  );
}

/** Status line and button of the "Updates" row. */
function UpdateRow() {
  const status = useUpdateStore((s) => s.status);
  const version = useUpdateStore((s) => s.version);
  const error = useUpdateStore((s) => s.error);
  const { check, install } = useUpdateStore.getState();
  const hint = !isTauri()
    ? "Available in the desktop app"
    : {
        idle: "New versions are installed from GitHub Releases",
        checking: "Checking…",
        uptodate: "You have the latest version",
        available: `Version ${version} is available`,
        downloading: `Downloading ${version}…`,
        error: `Could not check: ${error}`,
      }[status];
  return (
    <SettingRow title="Updates" hint={hint}>
      {status === "available" ? (
        <Button size="sm" variant="primary" onClick={() => void install()}>
          Update and restart
        </Button>
      ) : (
        <Button
          size="sm"
          icon={<RefreshCw size={13} className={status === "checking" ? "animate-spin" : undefined} />}
          disabled={!isTauri() || status === "checking" || status === "downloading"}
          onClick={() => void check()}
        >
          Check for updates
        </Button>
      )}
    </SettingRow>
  );
}

function AboutPanel() {
  return (
    <SettingsCard>
      <SettingRow title="Application" hint="Trajectory — Gmail, Outlook, IMAP and POP3">
        <span className="font-mono text-[13px] text-[var(--text-main)]">v{APP_VERSION}</span>
      </SettingRow>
      <Sep />
      <UpdateRow />
      <Sep />
      <SettingRow title="Credentials" hint="System keychain (Credential Manager / Keychain / Secret Service)" />
    </SettingsCard>
  );
}

type NavEntry = { id: SettingsTab; label: string; icon: LucideIcon };
const NAV: { group?: string; items: NavEntry[] }[] = [
  { items: [{ id: "appearance", label: "Appearance", icon: Palette }] },
  {
    group: "Mail",
    items: [
      { id: "accounts", label: "Accounts", icon: Users },
      { id: "add", label: "Add account", icon: Plus },
      { id: "oauth", label: "Sign-in apps", icon: KeyRound },
      { id: "general", label: "General", icon: SlidersHorizontal },
    ],
  },
];

const TITLES: Record<SettingsTab, [string, string]> = {
  appearance: ["Appearance", "Color theme of the application"],
  accounts: ["Accounts", "The mailboxes this app reads and sends from"],
  add: ["Add account", "Choose how you want to connect"],
  oauth: ["Sign-in apps", "Your own Google and Microsoft OAuth clients, kept in the system keychain"],
  general: ["General", "Reading and notifications"],
  about: ["About", "Application information"],
};

/** Settings — the same two-column window as Gravitation's. */
export function SettingsModal() {
  const open = useUiStore((s) => s.settingsOpen);
  const tab = useUiStore((s) => s.settingsTab);
  const { closeSettings, openSettings } = useUiStore.getState();

  const navButton = (it: NavEntry) => {
    const Icon = it.icon;
    const on = tab === it.id;
    return (
      <NavItem
        key={it.id}
        data-testid={`settings-tab-${it.id}`}
        aria-current={on ? "page" : undefined}
        active={on}
        className={on ? "text-[var(--text-main)]" : ""}
        icon={<Icon size={15} strokeWidth={1.7} className="shrink-0" />}
        onClick={() => openSettings(it.id)}
      >
        <span className="truncate">{it.label}</span>
      </NavItem>
    );
  };

  return (
    <Dialog
      open={open}
      onClose={closeSettings}
      label="Settings"
      className="h-[min(720px,calc(100vh-40px))] w-[min(1260px,calc(100vw-40px))] flex-row"
    >
      {/* Left column — sections, grouped; About pinned to the bottom. */}
      <div className="flex w-[200px] shrink-0 flex-col">
        <ScrollArea className="min-h-0 flex-1" innerClassName="flex flex-col gap-4 px-2 py-3">
          {NAV.map((grp, gi) => (
            <div key={grp.group ?? gi} className="flex flex-col gap-0.5">
              {grp.group && <div className="px-2.5 pb-1 text-[11px] text-[var(--text-dim)]">{grp.group}</div>}
              {grp.items.map(navButton)}
            </div>
          ))}
        </ScrollArea>
        <div className="px-2 pb-3">{navButton({ id: "about", label: "About", icon: Info })}</div>
      </div>

      {/* Right column — content on a card inset in the window. */}
      <div className="my-2 mr-2 flex min-w-0 flex-1 overflow-hidden rounded-[18px] border border-[var(--border)] bg-[var(--bg-app)]">
        <ScrollArea className="min-w-0 flex-1" innerClassName="min-w-0 px-8 py-6">
          <div className="mb-5 flex items-start">
            <div className="min-w-0">
              <div className="text-[17px] font-semibold text-[var(--text-main)]">{TITLES[tab][0]}</div>
              <div className="mt-0.5 text-[12.5px] text-[var(--text-dim)]">{TITLES[tab][1]}</div>
            </div>
            <IconButton label="Close settings" className="ml-auto" onClick={closeSettings}>
              <X size={14} />
            </IconButton>
          </div>

          {tab === "appearance" && <AppearancePanel />}
          {tab === "accounts" && <AccountsPanel />}
          {tab === "add" && <AddAccountPanel />}
          {tab === "oauth" && <OAuthAppsPanel />}
          {tab === "general" && <GeneralPanel />}
          {tab === "about" && <AboutPanel />}
        </ScrollArea>
      </div>
    </Dialog>
  );
}
