import { useEffect, useState } from "react";
import { ExternalLink, KeyRound } from "lucide-react";
import { api, errorMessage } from "@/lib/api";
import type { OAuthAppInfo, OAuthProviderId } from "@/lib/types";
import { useUiStore } from "@/store/uiStore";
import { Alert, Button, ConfirmBar, Field, Input, Pill, SettingsCard, Spinner } from "@/components";

const SOURCE_LABEL = {
  settings: "Saved in keychain",
  environment: "From environment",
  build: "Built into the app",
} as const;

const PROVIDERS: Record<
  OAuthProviderId,
  { title: string; idPlaceholder: string; needsSecret: boolean; console: string; steps: string[] }
> = {
  google: {
    title: "Google",
    idPlaceholder: "1234567890-abc….apps.googleusercontent.com",
    needsSecret: true,
    console: "https://console.cloud.google.com/apis/credentials",
    steps: [
      "Google Cloud Console → new project → enable the Gmail API.",
      "OAuth consent screen: External, Testing; add your address under Test users.",
      "Credentials → Create OAuth client ID → Desktop app.",
    ],
  },
  microsoft: {
    title: "Microsoft",
    idPlaceholder: "00000000-0000-0000-0000-000000000000",
    needsSecret: false,
    console: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    steps: [
      "Entra admin center → App registrations → New registration, any account type.",
      "Authentication → add platform Mobile and desktop → redirect URI http://localhost.",
      "Allow public client flows → Yes. Copy the Application (client) ID — there is no secret.",
    ],
  },
};

/** One provider's sign-in app: the client id (and Google's secret), saved to the OS keychain. */
function AppCard({ info, onChange }: { info: OAuthAppInfo; onChange: (next: OAuthAppInfo) => void }) {
  const p = PROVIDERS[info.provider];
  const pushToast = useUiStore((s) => s.pushToast);
  const saved = info.source === "settings";
  const [clientId, setClientId] = useState(saved ? (info.clientId ?? "") : "");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  const save = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const next = await api.setOAuthApp(info.provider, clientId.trim(), secret.trim() || null);
      setSecret("");
      onChange(next);
      pushToast(`${p.title} sign-in app saved`, "success");
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setConfirmClear(false);
    try {
      const next = await api.clearOAuthApp(info.provider);
      setClientId("");
      setSecret("");
      onChange(next);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <SettingsCard className="gap-4 py-4">
      <div className="flex items-center gap-2">
        <span className="text-[13.5px] font-medium text-[var(--text-main)]">{p.title}</span>
        {info.source ? <Pill tone={saved ? "success" : "accent"}>{SOURCE_LABEL[info.source]}</Pill> : <Pill>Not set up</Pill>}
        <Button size="xs" variant="ghost" className="ml-auto" icon={<ExternalLink size={12} />} onClick={() => void api.openExternal(p.console)}>
          Open console
        </Button>
      </div>

      <ol className="flex list-decimal flex-col gap-0.5 pl-5 text-[12px] text-[var(--text-dim)]">
        {p.steps.map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ol>

      {info.source && !saved && (
        <p className="text-[12px] text-[var(--text-dim)]">
          In use: <span className="font-mono text-[var(--text-muted)]">{info.clientId}</span>. Saving here takes precedence.
        </p>
      )}

      <form onSubmit={save} className="flex flex-col gap-3" aria-label={`${p.title} sign-in app`}>
        <Field label="Client ID">
          <Input
            mono
            autoComplete="off"
            aria-label={`${p.title} client ID`}
            placeholder={p.idPlaceholder}
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
          />
        </Field>
        {p.needsSecret && (
          <Field
            label="Client secret"
            hint={
              saved && info.secretHint
                ? `Saved: ${info.secretHint} — leave empty to keep it.`
                : "Google treats a desktop client's secret as non-confidential; it is kept in the keychain anyway and never shown again."
            }
          >
            <Input
              mono
              type="password"
              autoComplete="off"
              aria-label={`${p.title} client secret`}
              placeholder={saved && info.secretHint ? info.secretHint : "GOCSPX-…"}
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
            />
          </Field>
        )}
        {error && <Alert>{error}</Alert>}
        {confirmClear ? (
          <ConfirmBar
            message={`Forget the ${p.title} sign-in app? Signed-in accounts that use it stop syncing until it is set up again.`}
            confirmLabel="Forget"
            onConfirm={() => void clear()}
            onCancel={() => setConfirmClear(false)}
          />
        ) : (
          <div className="flex items-center gap-2">
            <Button type="submit" size="sm" variant="primary" disabled={busy || !clientId.trim()} icon={busy ? <Spinner size={12} /> : undefined}>
              Save
            </Button>
            {saved && (
              <Button size="sm" variant="ghost" onClick={() => setConfirmClear(true)}>
                Forget
              </Button>
            )}
          </div>
        )}
      </form>
    </SettingsCard>
  );
}

/**
 * Settings → Sign-in apps: your own Google / Microsoft OAuth clients, typed in
 * once and kept in the OS keychain — nothing is baked into the build.
 */
export function OAuthAppsPanel() {
  const [apps, setApps] = useState<OAuthAppInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.oauthApps().then(setApps, (e) => setError(errorMessage(e)));
  }, []);

  if (error) return <Alert>{error}</Alert>;
  if (!apps) return <Spinner size={16} className="text-[var(--text-dim)]" />;

  return (
    <div className="flex flex-col gap-3">
      <SettingsCard>
        <div className="flex items-start gap-3">
          <KeyRound size={15} className="mt-0.5 shrink-0 text-[var(--text-dim)]" />
          <p className="text-[12.5px] leading-relaxed text-[var(--text-muted)]">
            &ldquo;Sign in with Google / Microsoft&rdquo; needs an OAuth app of your own. Create it under a separate account if you like — your mailbox only signs
            in to it. The values are stored in the system keychain, never in the mail database, and the secret is never shown again. Accounts signed in
            with a different app have to sign in again after a change.
          </p>
        </div>
      </SettingsCard>
      {apps.map((a) => (
        <AppCard key={a.provider} info={a} onChange={(next) => setApps((cur) => cur?.map((x) => (x.provider === next.provider ? next : x)) ?? null)} />
      ))}
    </div>
  );
}
