import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ArrowLeft, CheckCircle2, Inbox, Mail, Send, Server, UserRound, XCircle } from "lucide-react";
import { api, errorMessage } from "@/lib/api";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import type { AccountKind, ConnectionReport, NewAccount, Security, ServerConfig } from "@/lib/types";
import { Alert, Button, Combobox, Field, Input, Pill, Spinner, cx } from "@/components";

function GoogleLogo() {
  return (
    <svg width="18" height="18" viewBox="0 0 48 48" aria-hidden="true">
      <path fill="#FFC107" d="M43.6 20.5H42V20H24v8h11.3C33.7 32.7 29.2 36 24 36c-6.6 0-12-5.4-12-12s5.4-12 12-12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 12.9 4 4 12.9 4 24s8.9 20 20 20 20-8.9 20-20c0-1.3-.1-2.4-.4-3.5z" />
      <path fill="#FF3D00" d="m6.3 14.7 6.6 4.8C14.7 15.1 19 12 24 12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 16.3 4 9.7 8.3 6.3 14.7z" />
      <path fill="#4CAF50" d="M24 44c5.2 0 9.9-2 13.4-5.2l-6.2-5.2C29.2 35.1 26.7 36 24 36c-5.2 0-9.6-3.3-11.3-8l-6.5 5C9.5 39.6 16.2 44 24 44z" />
      <path fill="#1976D2" d="M43.6 20.5H42V20H24v8h11.3c-.8 2.2-2.2 4.2-4.1 5.6l6.2 5.2C37 39.2 44 34 44 24c0-1.3-.1-2.4-.4-3.5z" />
    </svg>
  );
}

function MicrosoftLogo() {
  return (
    <svg width="16" height="16" viewBox="0 0 23 23" aria-hidden="true">
      <path fill="#f35325" d="M1 1h10v10H1z" />
      <path fill="#81bc06" d="M12 1h10v10H12z" />
      <path fill="#05a6f0" d="M1 12h10v10H1z" />
      <path fill="#ffba08" d="M12 12h10v10H12z" />
    </svg>
  );
}

const PROVIDERS: { kind: AccountKind; title: string; subtitle: string; icon: React.ReactNode }[] = [
  { kind: "gmail", title: "Google", subtitle: "Gmail & Workspace via OAuth", icon: <GoogleLogo /> },
  { kind: "outlook", title: "Microsoft", subtitle: "Outlook.com & Office 365", icon: <MicrosoftLogo /> },
  { kind: "imap", title: "IMAP", subtitle: "Any provider · IMAP + SMTP", icon: <Server size={16} /> },
  { kind: "pop3", title: "POP3", subtitle: "Download mail · POP3 + SMTP", icon: <Mail size={16} /> },
];

const SECURITY_OPTIONS: { value: Security; label: string }[] = [
  { value: "tls", label: "SSL/TLS" },
  { value: "start_tls", label: "STARTTLS" },
  { value: "none", label: "None (insecure)" },
];

const PERIOD_OPTIONS = [
  { value: "0", label: "All messages" },
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "90", label: "Last 90 days" },
  { value: "365", label: "Last year" },
];

/** A titled block of fields — the same card as Gravitation's edit forms. */
function Card({ icon: Icon, title, aside, children }: { icon: typeof Server; title: string; aside?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-3 rounded-2xl border border-[var(--border-soft)] bg-[var(--bg-surface)] p-4">
      <div className="flex min-h-5 items-center gap-2">
        <Icon size={14} strokeWidth={1.7} className="shrink-0 text-[var(--text-dim)]" />
        <span className="text-[12.5px] font-medium text-[var(--text-main)]">{title}</span>
        {aside && <span className="ml-auto flex min-w-0 items-center text-[11px] text-[var(--text-dim)]">{aside}</span>}
      </div>
      {children}
    </section>
  );
}

function ServerFields({
  title,
  icon,
  value,
  onChange,
  prefix,
}: {
  title: string;
  icon: typeof Server;
  value: ServerConfig;
  onChange: (v: ServerConfig) => void;
  prefix: string;
}) {
  return (
    <Card icon={icon} title={title}>
      <div className="grid grid-cols-[1fr_88px_160px] gap-2">
        <Field label="Host">
          <Input aria-label={`${prefix} host`} mono placeholder="mail.example.com" value={value.host} onChange={(e) => onChange({ ...value, host: e.target.value })} />
        </Field>
        <Field label="Port">
          <Input
            aria-label={`${prefix} port`}
            mono
            inputMode="numeric"
            value={value.port || ""}
            onChange={(e) => onChange({ ...value, port: Number(e.target.value.replace(/\D/g, "")) || 0 })}
          />
        </Field>
        <div className="flex flex-col">
          <span className="mb-1.5 block text-[11px] font-medium text-[var(--text-muted)]">Security</span>
          <Combobox
            searchable={false}
            value={value.security}
            options={SECURITY_OPTIONS}
            onChange={(security) => onChange({ ...value, security: security as Security })}
          />
        </div>
      </div>
    </Card>
  );
}

const emptyServer = (port: number): ServerConfig => ({ host: "", port, security: "tls" });

function ManualForm({ kind, onBack, onOAuth }: { kind: "imap" | "pop3"; onBack: () => void; onOAuth: (k: AccountKind) => void }) {
  const reloadAccounts = useMailStore((s) => s.reloadAccounts);
  const { pushToast, closeSettings } = useUiStore.getState();
  const [form, setForm] = useState({ displayName: "", email: "", password: "", username: "" });
  const [incoming, setIncoming] = useState<ServerConfig>(emptyServer(kind === "imap" ? 993 : 995));
  const [outgoing, setOutgoing] = useState<ServerConfig>(emptyServer(465));
  const [period, setPeriod] = useState("30");
  const [detected, setDetected] = useState<{ provider: string | null; suggested: AccountKind | null } | null>(null);
  const [report, setReport] = useState<ConnectionReport | null>(null);
  const [busy, setBusy] = useState<"test" | "add" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const autodetect = async () => {
    if (!form.email.includes("@")) return;
    try {
      const d = await api.autodetect(form.email);
      setIncoming((cur) => (cur.host ? cur : kind === "imap" ? d.imap : d.pop3));
      setOutgoing((cur) => (cur.host ? cur : d.smtp));
      setDetected({ provider: d.provider, suggested: d.suggestedKind });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const payload = (): NewAccount => ({
    kind,
    displayName: form.displayName.trim(),
    email: form.email.trim(),
    incoming,
    outgoing,
    username: form.username.trim() || null,
    password: form.password,
    pop3DownloadDays: kind === "pop3" && period !== "0" ? Number(period) : null,
  });

  const test = async () => {
    setBusy("test");
    setError(null);
    setReport(null);
    try {
      setReport(await api.testConnection(payload()));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy("add");
    setError(null);
    try {
      const account = await api.addAccount(payload(), true);
      await reloadAccounts(account.id);
      pushToast(`${account.email} added`, "success");
      closeSettings();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <form onSubmit={add} className="flex flex-col gap-3" aria-label={`${kind.toUpperCase()} account`}>
      <div className="flex items-center gap-2">
        <Button size="sm" variant="ghost" onClick={onBack} icon={<ArrowLeft size={13} />}>
          Back
        </Button>
        <span className="text-[13px] font-medium text-[var(--text-main)]">{kind === "imap" ? "IMAP account" : "POP3 account"}</span>
        {detected?.provider && <Pill tone="accent">Detected: {detected.provider}</Pill>}
      </div>

      {detected?.suggested && (
        <Alert tone="info">
          <div className="flex items-center gap-3">
            <span className="flex-1">This provider supports secure sign-in without an app password.</span>
            <Button size="sm" variant="primary" onClick={() => onOAuth(detected.suggested!)}>
              Use {detected.provider} sign-in
            </Button>
          </div>
        </Alert>
      )}

      <Card icon={UserRound} title="Account">
        <div className="grid grid-cols-2 gap-3">
          <Field label="Name">
            <Input placeholder="Jane Doe" value={form.displayName} onChange={(e) => setForm({ ...form, displayName: e.target.value })} />
          </Field>
          <Field label="Email">
            <Input
              aria-label="Email"
              type="email"
              required
              placeholder="jane@example.com"
              value={form.email}
              onChange={(e) => setForm({ ...form, email: e.target.value })}
              onBlur={() => void autodetect()}
            />
          </Field>
          <Field label="Password" hint="Stored in the system keychain, never on disk.">
            <Input
              aria-label="Password"
              type="password"
              required
              autoComplete="current-password"
              value={form.password}
              onChange={(e) => setForm({ ...form, password: e.target.value })}
            />
          </Field>
          <Field label="Username" hint="Defaults to the email address.">
            <Input aria-label="Username" value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} />
          </Field>
        </div>
      </Card>

      <ServerFields title={kind === "imap" ? "Incoming · IMAP" : "Incoming · POP3"} icon={Inbox} prefix="Incoming" value={incoming} onChange={setIncoming} />
      <ServerFields title="Outgoing · SMTP" icon={Send} prefix="Outgoing" value={outgoing} onChange={setOutgoing} />

      {kind === "pop3" && (
        <Card icon={Mail} title="Download period">
          <div className="w-[240px]">
            <Combobox searchable={false} value={period} options={PERIOD_OPTIONS} onChange={setPeriod} />
          </div>
        </Card>
      )}

      <AnimatePresence>
        {report && (
          <motion.div initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }} className="flex flex-col gap-1.5 px-1" data-testid="connection-report">
            {(
              [
                ["Incoming", report.incomingOk, report.incomingError],
                ["Outgoing", report.outgoingOk, report.outgoingError],
              ] as const
            ).map(([label, ok, err]) => (
              <div key={label} className={cx("flex items-center gap-2 text-[12px]", ok ? "text-[var(--diff-add)]" : "text-[var(--diff-del)]")}>
                {ok ? <CheckCircle2 size={14} /> : <XCircle size={14} />}
                <span className="font-medium">{label}:</span>
                <span className="text-[var(--text-muted)]">{ok ? "Connected" : err}</span>
              </div>
            ))}
          </motion.div>
        )}
      </AnimatePresence>

      {error && <Alert>{error}</Alert>}

      <div className="flex justify-end gap-2 pt-1">
        <Button onClick={() => void test()} disabled={busy !== null} icon={busy === "test" ? <Spinner size={13} /> : undefined}>
          Test connection
        </Button>
        <Button variant="primary" type="submit" disabled={busy !== null} icon={busy === "add" ? <Spinner size={13} /> : undefined}>
          Add account
        </Button>
      </div>
    </form>
  );
}

export function AddAccountPanel() {
  const [kind, setKind] = useState<AccountKind | null>(null);
  const [waiting, setWaiting] = useState<AccountKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reloadAccounts = useMailStore((s) => s.reloadAccounts);
  const { pushToast, closeSettings } = useUiStore.getState();

  const oauth = async (k: AccountKind) => {
    setWaiting(k);
    setError(null);
    try {
      const account = await api.addOAuthAccount(k);
      await reloadAccounts(account.id);
      pushToast(`${account.email} connected`, "success");
      closeSettings();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWaiting(null);
    }
  };

  if (kind === "imap" || kind === "pop3") return <ManualForm kind={kind} onBack={() => setKind(null)} onOAuth={(k) => void oauth(k)} />;

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3">
        {PROVIDERS.map((p) => (
          <button
            key={p.kind}
            type="button"
            data-testid={`provider-${p.kind}`}
            disabled={waiting !== null}
            onClick={() => (p.kind === "gmail" || p.kind === "outlook" ? void oauth(p.kind) : setKind(p.kind))}
            className="flex items-center gap-3 rounded-2xl border border-[var(--border-soft)] bg-[var(--bg-surface)] p-4 text-left transition-colors hover:border-[var(--border)] hover:bg-[var(--hover-bg)] disabled:opacity-50"
          >
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--bg-input)] text-[var(--text-main)]">
              {waiting === p.kind ? <Spinner size={16} /> : p.icon}
            </span>
            <span className="flex min-w-0 flex-col">
              <span className="text-[13px] font-medium text-[var(--text-main)]">{waiting === p.kind ? "Waiting for browser…" : p.title}</span>
              <span className="truncate text-[11.5px] text-[var(--text-dim)]">{p.subtitle}</span>
            </span>
          </button>
        ))}
      </div>
      {waiting && <p className="px-1 text-[12px] text-[var(--text-dim)]">Complete the sign-in in your browser, then return here.</p>}
      {error && (
        <Alert>
          <div className="flex items-center gap-3">
            <span className="flex-1">{error}</span>
            {/not set up|not configured/i.test(error) && (
              <Button size="sm" variant="primary" onClick={() => useUiStore.getState().openSettings("oauth")}>
                Set up sign-in
              </Button>
            )}
          </div>
        </Alert>
      )}
    </div>
  );
}
