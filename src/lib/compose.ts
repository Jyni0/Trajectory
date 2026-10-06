import type { Email, OutgoingAttachment } from "./types";
import { displayName, formatAddress, formatFullDate } from "./format";

/** Editable composer state (string fields, comma-separated recipients). */
export interface ComposerDraft {
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  html: string;
  attachments: OutgoingAttachment[];
  inReplyTo: string | null;
  draftId: string | null;
}

export function emptyDraft(accountId: string): ComposerDraft {
  return { accountId, to: [], cc: [], bcc: [], subject: "", html: "", attachments: [], inReplyTo: null, draftId: null };
}

const escapeHtml = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

function prefixed(prefix: "Re" | "Fwd", subject: string): string {
  const re = new RegExp(`^\\s*${prefix}:\\s*`, "i");
  return re.test(subject) ? subject : `${prefix}: ${subject}`;
}

function quotedBody(email: Email): string {
  return email.bodyHtml ?? `<div style="white-space:pre-wrap">${escapeHtml(email.bodyText ?? "")}</div>`;
}

export function replyDraft(email: Email, accountEmail: string, all = false): ComposerDraft {
  const replyTo = email.replyTo.length ? email.replyTo : [email.from];
  const self = accountEmail.toLowerCase();
  const others = all
    ? [...email.to, ...email.cc].filter((a) => a.address.toLowerCase() !== self && !replyTo.some((r) => r.address === a.address))
    : [];
  return {
    ...emptyDraft(email.accountId),
    to: replyTo.map(formatAddress),
    cc: others.map(formatAddress),
    subject: prefixed("Re", email.subject),
    html: `<p><br></p><p>On ${escapeHtml(formatFullDate(email.date))}, ${escapeHtml(displayName(email.from))} wrote:</p><blockquote>${quotedBody(email)}</blockquote>`,
    inReplyTo: email.messageId,
  };
}

export function forwardDraft(email: Email): ComposerDraft {
  const header = [
    "---------- Forwarded message ----------",
    `From: ${formatAddress(email.from)}`,
    `Date: ${formatFullDate(email.date)}`,
    `Subject: ${email.subject}`,
    `To: ${email.to.map(formatAddress).join(", ")}`,
  ]
    .map(escapeHtml)
    .join("<br>");
  return {
    ...emptyDraft(email.accountId),
    subject: prefixed("Fwd", email.subject),
    html: `<p><br></p><p>${header}</p>${quotedBody(email)}`,
  };
}

/** Restores a saved draft into the composer. */
export function draftFromEmail(email: Email): ComposerDraft {
  return {
    ...emptyDraft(email.accountId),
    to: email.to.map(formatAddress),
    cc: email.cc.map(formatAddress),
    subject: email.subject,
    html: email.bodyHtml ?? escapeHtml(email.bodyText ?? "").replace(/\n/g, "<br>"),
    inReplyTo: email.inReplyTo,
    draftId: email.id,
  };
}

export function htmlToPlainText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
  doc.querySelectorAll("p,div,li,blockquote").forEach((el) => el.append("\n"));
  return (doc.body.textContent ?? "").replace(/\n{3,}/g, "\n\n").trim();
}

export function splitRecipients(value: string): string[] {
  return value
    .split(/[,;\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

export function isValidRecipient(value: string): boolean {
  const m = value.match(/<([^>]+)>\s*$/);
  const address = (m ? m[1] : value).trim();
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(address);
}

export function fileToAttachment(file: File): Promise<OutgoingAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const result = String(reader.result);
      resolve({
        filename: file.name,
        mimeType: file.type || "application/octet-stream",
        dataBase64: result.slice(result.indexOf(",") + 1),
      });
    };
    reader.readAsDataURL(file);
  });
}
