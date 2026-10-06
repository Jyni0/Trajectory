import { useEffect, useState } from "react";
import { BadgeCheck } from "lucide-react";
import { api } from "@/lib/api";
import { displayName } from "@/lib/format";
import type { Address, SenderLogo } from "@/lib/types";
import { useUiStore } from "@/store/uiStore";
import { Avatar } from "@/components";

const NONE: SenderLogo = { svg: null, verified: false, issuer: null };

/** One lookup per domain per session; the backend caches across sessions. */
const logos = new Map<string, SenderLogo | Promise<SenderLogo>>();

function fetchLogo(domain: string): Promise<SenderLogo> {
  const hit = logos.get(domain);
  if (hit) return Promise.resolve(hit);
  const p = api.senderLogo(domain).then(
    (logo) => (logos.set(domain, logo), logo),
    // Offline etc.: forget the failure so a later view tries again.
    () => (logos.delete(domain), NONE),
  );
  logos.set(domain, p);
  return p;
}

/** Drops the session cache (tests). */
export function resetSenderLogos() {
  logos.clear();
}

/**
 * BIMI logo of a sender domain. `logoDomain` is only set by the backend when
 * the message passed DMARC for it, so a forged From never gets a logo.
 */
export function useSenderLogo(logoDomain: string | null | undefined): SenderLogo {
  const enabled = useUiStore((s) => s.prefs.senderLogos);
  const domain = enabled && logoDomain ? logoDomain.toLowerCase() : null;
  const cached = domain ? logos.get(domain) : undefined;
  const [logo, setLogo] = useState<SenderLogo>(cached && !(cached instanceof Promise) ? cached : NONE);

  useEffect(() => {
    if (!domain) return setLogo(NONE);
    let alive = true;
    void fetchLogo(domain).then((l) => alive && setLogo(l));
    return () => {
      alive = false;
    };
  }, [domain]);
  return domain ? logo : NONE;
}

/** The sender's avatar: their brand logo or site icon, initials otherwise. */
export function SenderAvatar({ from, logoDomain, size, color }: { from: Address; logoDomain: string | null; size: number; color?: string }) {
  const logo = useSenderLogo(logoDomain);
  return <Avatar name={displayName(from)} email={from.address} size={size} color={color} src={logo.svg} />;
}

/** Domain of an address (`alex@mail.example.com` → `mail.example.com`). */
export function domainOf(address: string): string | null {
  const d = address.split("@")[1]?.trim();
  return d || null;
}

/** Check mark before the sender name when the brand logo is certified (BIMI Verified Mark Certificate). */
export function VerifiedMark({ logoDomain, size = 13 }: { logoDomain: string | null; size?: number }) {
  const logo = useSenderLogo(logoDomain);
  if (!logo.svg || !logo.verified) return null;
  const by = logo.issuer ? ` by ${logo.issuer}` : "";
  return (
    <span
      data-testid="verified-sender"
      title={`${logoDomain} — brand logo verified${by} (BIMI Verified Mark Certificate)`}
      aria-label="Verified sender"
      className="inline-flex shrink-0 items-center self-center text-[var(--accent)]"
    >
      <BadgeCheck size={size} strokeWidth={2} />
    </span>
  );
}
