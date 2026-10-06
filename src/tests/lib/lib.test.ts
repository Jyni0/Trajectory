import { describe, expect, it } from "vitest";
import { avatarColor, formatListDate, formatSize, hasRemoteContent, initials } from "@/lib/format";
import { draftFromEmail, forwardDraft, htmlToPlainText, isValidRecipient, replyDraft, splitRecipients } from "@/lib/compose";
import type { Email } from "@/lib/types";

const email: Email = {
  id: "e1",
  accountId: "acc",
  folderId: "inbox",
  from: { name: "Anna", address: "anna@x.io" },
  subject: "Plans",
  snippet: "",
  date: 1_788_256_800,
  isRead: true,
  isFlagged: false,
  hasAttachments: false,
  to: [{ name: "Me", address: "me@x.io" }, { name: null, address: "bob@x.io" }],
  cc: [{ name: "Carol", address: "carol@x.io" }],
  replyTo: [],
  bodyHtml: null,
  bodyText: "Line <1>\nLine 2",
  messageId: "<m1@x.io>",
  inReplyTo: null,
  attachments: [],
  tags: [],
  hasNote: false,
  note: null,
  logoDomain: null,
};

describe("format", () => {
  it("formats sizes", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(2048)).toBe("2.0 KB");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
  });

  it("derives initials from names and addresses", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("john.smith@example.com")).toBe("JS");
    expect(initials("GitHub")).toBe("GI");
    expect(initials("Иван Петров")).toBe("ИП");
    expect(initials("")).toBe("?");
  });

  it("gives stable avatar colors", () => {
    expect(avatarColor("a@b.c")).toBe(avatarColor("a@b.c"));
  });

  it("formats list dates relative to now", () => {
    const now = new Date(2026, 9, 2, 15, 0);
    const today = new Date(2026, 9, 2, 9, 5).getTime() / 1000;
    const lastYear = new Date(2025, 0, 3).getTime() / 1000;
    expect(formatListDate(today, now)).toMatch(/09|9/);
    expect(formatListDate(lastYear, now)).toMatch(/2025/);
  });

  it("detects remote content", () => {
    expect(hasRemoteContent('<img src="https://t.co/p.gif">')).toBe(true);
    expect(hasRemoteContent('<div style="background:url(http://x/y.png)">')).toBe(true);
    expect(hasRemoteContent('<img src="data:image/png;base64,AA">')).toBe(false);
  });
});

describe("compose helpers", () => {
  it("builds a reply with quoted, escaped text", () => {
    const d = replyDraft(email, "me@x.io");
    expect(d.to).toEqual(["Anna <anna@x.io>"]);
    expect(d.subject).toBe("Re: Plans");
    expect(d.inReplyTo).toBe("<m1@x.io>");
    expect(d.html).toContain("<blockquote>");
    expect(d.html).toContain("Line &lt;1&gt;");
    expect(replyDraft({ ...email, subject: "RE: Plans" }, "me@x.io").subject).toBe("RE: Plans");
  });

  it("reply-all includes other recipients but not myself", () => {
    const d = replyDraft(email, "ME@x.io", true);
    expect(d.cc).toEqual(["bob@x.io", "Carol <carol@x.io>"]);
  });

  it("forwards with a header block and no recipients", () => {
    const d = forwardDraft(email);
    expect(d.to).toEqual([]);
    expect(d.subject).toBe("Fwd: Plans");
    expect(d.html).toContain("Forwarded message");
  });

  it("restores drafts for editing", () => {
    const d = draftFromEmail({ ...email, bodyHtml: "<p>hi</p>" });
    expect(d.draftId).toBe("e1");
    expect(d.html).toBe("<p>hi</p>");
    expect(d.to).toHaveLength(2);
  });

  it("converts editor HTML to plain text", () => {
    expect(htmlToPlainText("<p>Hello <b>world</b></p><div>second<br>line</div>")).toBe("Hello world\nsecond\nline");
  });

  it("splits and validates recipients", () => {
    expect(splitRecipients("a@b.co, c@d.co;\n e@f.co ,")).toEqual(["a@b.co", "c@d.co", "e@f.co"]);
    expect(isValidRecipient("Jane <jane@x.io>")).toBe(true);
    expect(isValidRecipient("jane@x.io")).toBe(true);
    expect(isValidRecipient("jane@")).toBe(false);
    expect(isValidRecipient("not an email")).toBe(false);
  });
});
