import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { EmailList } from "@/components/mail/EmailList";
import { MailReader } from "@/components/mail/MailReader";
import { Composer, composerKey } from "@/components/mail/Composer";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { emptyDraft } from "@/lib/compose";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

let backend: MockBackend;

/** Mounts the composer the way App does: from the store, keyed by draft. */
function ComposerHost() {
  const draft = useUiStore((s) => s.composer);
  return draft ? <Composer key={composerKey(draft)} initial={draft} /> : null;
}

beforeEach(async () => {
  backend = setupBackend();
  await useMailStore.getState().init();
});

describe("EmailList", () => {
  it("renders virtualized rows with sender, subject and unread markers", () => {
    render(<EmailList />);
    const rows = screen.getAllByTestId("email-row");
    expect(rows.length).toBe(12);
    expect(within(rows[0]).getByText("Weekly product sync — agenda")).toBeInTheDocument();
    expect(rows.filter((r) => r.dataset.unread).length).toBe(useMailStore.getState().emails.filter((e) => !e.isRead).length);
    expect(screen.getByText("12 messages")).toBeInTheDocument();
  });

  it("filters with the segmented control", async () => {
    render(<EmailList />);
    await userEvent.click(screen.getByRole("radio", { name: "Flagged" }));
    await waitFor(() => expect(screen.getByRole("radio", { name: "Flagged" })).toHaveAttribute("aria-checked", "true"));
    await waitFor(() => expect(screen.getAllByTestId("email-row").length).toBe(useMailStore.getState().emails.length));
    expect(useMailStore.getState().emails.every((e) => e.isFlagged)).toBe(true);
  });

  it("searches with debounce and shows an empty state", async () => {
    render(<EmailList />);
    await userEvent.type(screen.getByRole("textbox", { name: "Search mail" }), "zzzz-nothing");
    await waitFor(() => expect(screen.getByText("No results")).toBeInTheDocument(), { timeout: 2000 });
    await userEvent.click(screen.getByRole("button", { name: "Clear search" }));
    await waitFor(() => expect(screen.getAllByTestId("email-row").length).toBe(12), { timeout: 2000 });
  });

  it("selects a message on click and toggles the flag from the row", async () => {
    render(<EmailList />);
    const row = screen.getAllByTestId("email-row")[0];
    await userEvent.click(row);
    await waitFor(() => expect(useMailStore.getState().activeEmail?.subject).toBe("Weekly product sync — agenda"));
    const flagBtn = within(screen.getAllByTestId("email-row")[0]).getByRole("button", { name: /flag/i });
    const before = useMailStore.getState().emails[0].isFlagged;
    await userEvent.click(flagBtn);
    expect(useMailStore.getState().emails[0].isFlagged).toBe(!before);
  });
});

describe("MailReader", () => {
  it("shows an empty state with no selection", () => {
    render(<MailReader />);
    expect(screen.getByText("Select a message to read")).toBeInTheDocument();
  });

  it("renders headers, sandboxed HTML body and actions", async () => {
    const first = useMailStore.getState().emails[0];
    await useMailStore.getState().selectEmail(first.id);
    render(<MailReader />);
    expect(screen.getByTestId("reader-subject")).toHaveTextContent(first.subject);
    const body = screen.getByTestId("email-body");
    expect(body.tagName).toBe("IFRAME");
    expect(body).toHaveAttribute("sandbox", "allow-same-origin allow-popups");
    expect(body.getAttribute("srcdoc")).toContain("default-src 'none'");
    expect(screen.getByRole("toolbar", { name: "Message actions" })).toBeInTheDocument();
  });

  it("strips scripts and blocks remote images until allowed", async () => {
    const mail = backend.deliver(useMailStore.getState().activeAccountId!, {
      subject: "Tracked",
      bodyHtml: '<p>Hi</p><img src="https://tracker.example/p.gif"><script>alert(1)</script>',
    });
    await useMailStore.getState().loadEmails();
    await useMailStore.getState().selectEmail(mail.id);
    render(<MailReader />);
    const srcdoc = screen.getByTestId("email-body").getAttribute("srcdoc")!;
    expect(srcdoc).not.toContain("<script");
    expect(srcdoc).toContain("img-src data: blob:;");
    await userEvent.click(screen.getByRole("button", { name: "Load images" }));
    expect(screen.getByTestId("email-body").getAttribute("srcdoc")).toContain("https:");
  });

  it("reply opens the composer prefilled", async () => {
    const first = useMailStore.getState().emails[0];
    await useMailStore.getState().selectEmail(first.id);
    render(<MailReader />);
    await userEvent.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    const draft = useUiStore.getState().composer!;
    expect(draft.subject).toBe(`Re: ${first.subject}`);
    expect(draft.to[0]).toContain(first.from.address);
  });

  it("archive and delete remove the message from the list", async () => {
    const [a, b] = useMailStore.getState().emails;
    await useMailStore.getState().selectEmail(a.id);
    render(<MailReader />);
    await userEvent.click(screen.getByRole("button", { name: "Archive" }));
    await waitFor(() => expect(useMailStore.getState().selectedEmailId).toBe(b.id));
    await waitFor(() => expect(screen.getByTestId("reader-subject")).toHaveTextContent(b.subject));
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(useMailStore.getState().emails.some((e) => e.id === b.id)).toBe(false));
  });

  it("lists attachments and saves them", async () => {
    const withAtt = useMailStore.getState().emails.find((e) => e.hasAttachments)!;
    await useMailStore.getState().selectEmail(withAtt.id);
    render(<MailReader />);
    const region = screen.getByRole("region", { name: "Attachments" });
    await userEvent.click(within(region).getAllByRole("button")[0]);
    await waitFor(() => expect(useUiStore.getState().toasts.at(-1)?.message).toMatch(/^Saved to/));
  });
});

describe("Composer", () => {
  const open = () => useUiStore.getState().openComposer(emptyDraft(useMailStore.getState().activeAccountId!));

  it("adds recipient chips and validates before sending", async () => {
    open();
    render(<ComposerHost />);
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Add at least one recipient.");

    const to = screen.getByRole("textbox", { name: "To" });
    await userEvent.type(to, "broken{Enter}");
    expect(screen.getByTestId("recipient-chip")).toHaveAttribute("data-invalid", "true");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.getByRole("alert")).toHaveTextContent('"broken" is not a valid address.');

    await userEvent.click(screen.getByRole("button", { name: "Remove broken" }));
    expect(screen.queryByTestId("recipient-chip")).not.toBeInTheDocument();
  });

  it("sends a message and files it in Sent", async () => {
    open();
    render(<ComposerHost />);
    await userEvent.type(screen.getByRole("textbox", { name: "To" }), "friend@example.com,");
    await userEvent.type(screen.getByRole("textbox", { name: "Subject" }), "Hello there");
    const editor = screen.getByTestId("composer-editor");
    editor.innerHTML = "<p>Body text</p>";
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(useUiStore.getState().composer).toBeNull());
    const sent = backend.db.emails.find((e) => e.subject === "Hello there")!;
    expect(sent.to[0].address).toBe("friend@example.com");
    expect(sent.bodyText).toBe("Body text");
    expect(backend.db.folders.find((f) => f.id === sent.folderId)?.role).toBe("sent");
  });

  it("attaches files", async () => {
    open();
    render(<ComposerHost />);
    const file = new File(["hello"], "notes.txt", { type: "text/plain" });
    await userEvent.upload(screen.getByTestId("composer-file-input"), file);
    await waitFor(() => expect(screen.getByText("notes.txt")).toBeInTheDocument());
    expect(useUiStore.getState().composer).not.toBeNull();
  });

  it("saves a draft when closed with content", async () => {
    open();
    render(<ComposerHost />);
    await userEvent.type(screen.getByRole("textbox", { name: "Subject" }), "Unfinished");
    await userEvent.click(screen.getByRole("button", { name: "Close composer" }));
    await waitFor(() => expect(useUiStore.getState().composer).toBeNull());
    const draft = backend.db.emails.find((e) => e.subject === "Unfinished")!;
    expect(backend.db.folders.find((f) => f.id === draft.folderId)?.role).toBe("drafts");
  });

  it("discarding an empty message saves nothing", async () => {
    open();
    render(<ComposerHost />);
    const before = backend.db.emails.length;
    await userEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(useUiStore.getState().composer).toBeNull();
    expect(backend.db.emails.length).toBe(before);
  });
});

describe("App", () => {
  it("shows onboarding without accounts and adds an IMAP account end-to-end", async () => {
    backend = setupBackend({ withDemoAccount: false, seedCount: 5 });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Add your first account" }));
    await userEvent.click(screen.getByTestId("provider-imap"));

    await userEvent.type(screen.getByLabelText("Email"), "jane@corp.example");
    await userEvent.tab(); // blur → autodetect
    await waitFor(() => expect(screen.getByLabelText("Incoming host")).toHaveValue("imap.corp.example"));
    expect(screen.getByLabelText("Outgoing host")).toHaveValue("smtp.corp.example");

    await userEvent.type(screen.getByLabelText("Password"), "wrong");
    await userEvent.click(screen.getByRole("button", { name: "Test connection" }));
    const report = await screen.findByTestId("connection-report");
    expect(report).toHaveTextContent("invalid credentials");

    await userEvent.clear(screen.getByLabelText("Password"));
    await userEvent.type(screen.getByLabelText("Password"), "correct-horse");
    await userEvent.click(within(screen.getByRole("form", { name: "IMAP account" })).getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(screen.getAllByTestId("email-row").length).toBe(5));
    expect(screen.getByRole("heading", { name: "Inbox" })).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("autodetect suggests OAuth for Gmail addresses", async () => {
    backend = setupBackend({ withDemoAccount: false });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Add your first account" }));
    await userEvent.click(screen.getByTestId("provider-imap"));
    await userEvent.type(screen.getByLabelText("Email"), "me@gmail.com");
    await userEvent.tab();
    expect(await screen.findByText("Detected: Google")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Use Google sign-in" }));
    await waitFor(() => expect(useMailStore.getState().accounts[0]?.kind).toBe("gmail"));
  });

  it("reacts to mail:new events from the sync engine", async () => {
    render(<App />);
    await screen.findAllByTestId("email-row");
    backend.deliver(useMailStore.getState().activeAccountId!, { subject: "Fresh off the wire", from: { name: "Zed", address: "zed@x.io" } });
    await waitFor(() => expect(screen.getByText("Fresh off the wire")).toBeInTheDocument());
    expect(useUiStore.getState().toasts.some((t) => t.message.includes("Zed"))).toBe(true);
  });

  it("supports keyboard shortcuts", async () => {
    render(<App />);
    await screen.findAllByTestId("email-row");
    await userEvent.keyboard("j");
    await waitFor(() => expect(useMailStore.getState().selectedEmailId).toBe(useMailStore.getState().emails[0].id));
    await userEvent.keyboard("c");
    expect(useUiStore.getState().composer).not.toBeNull();
  });
});

describe("Folder filters", () => {
  it("All / Unread / Flagged only where mail is received", async () => {
    render(<EmailList />);
    expect(screen.getByRole("radio", { name: "Unread" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: "Unread" }));
    await waitFor(() => expect(useMailStore.getState().filter).toBe("unread"));

    for (const role of ["sent", "drafts", "archive", "trash"] as const) {
      await act(() => useMailStore.getState().selectFolderByRole(role));
      expect(screen.queryByRole("radio", { name: "Unread" })).not.toBeInTheDocument();
      // A hidden filter is dropped, so the folder shows everything.
      expect(useMailStore.getState().filter).toBe("all");
    }
    await act(() => useMailStore.getState().selectFolderByRole("inbox"));
    expect(screen.getByRole("radio", { name: "All" })).toHaveAttribute("aria-checked", "true");
  });
});

describe("Leaving the composer", () => {
  function Panes() {
    const composer = useUiStore((s) => s.composer);
    return (
      <>
        <EmailList />
        {composer ? <Composer key={composerKey(composer)} initial={composer} /> : <MailReader />}
      </>
    );
  }

  it("opening a message saves what was written to Drafts", async () => {
    const accountId = useMailStore.getState().activeAccountId!;
    render(<Panes />);
    act(() => useUiStore.getState().openComposer(emptyDraft(accountId)));
    await userEvent.type(await screen.findByRole("textbox", { name: "Subject" }), "Half-written");
    const drafts = () => backend.db.emails.filter((e) => e.subject === "Half-written").length;

    await userEvent.click(screen.getAllByTestId("email-row")[1]);
    await waitFor(() => expect(useUiStore.getState().composer).toBeNull());
    expect(drafts()).toBe(1);
    expect(await screen.findByTestId("reader-subject")).toHaveTextContent(useMailStore.getState().emails[1].subject);
  });

  it("an untouched composer just closes", async () => {
    const accountId = useMailStore.getState().activeAccountId!;
    render(<Panes />);
    act(() => useUiStore.getState().openComposer(emptyDraft(accountId)));
    await screen.findByRole("region", { name: "Composer" });
    const before = backend.db.emails.length;
    await userEvent.click(screen.getAllByTestId("email-row")[0]);
    await waitFor(() => expect(useUiStore.getState().composer).toBeNull());
    expect(backend.db.emails.length).toBe(before);
    expect(await screen.findByTestId("reader-subject")).toBeInTheDocument();
  });

  it("an unchanged reply is not kept as a draft", async () => {
    const first = useMailStore.getState().emails[0];
    await useMailStore.getState().selectEmail(first.id);
    render(<Panes />);
    await userEvent.click(screen.getByRole("button", { name: "Reply all" }));
    await screen.findByRole("region", { name: "Composer" });
    const before = backend.db.emails.length;
    await userEvent.click(screen.getAllByTestId("email-row")[2]);
    await waitFor(() => expect(useUiStore.getState().composer).toBeNull());
    expect(backend.db.emails.length).toBe(before);
  });
});

describe("HtmlViewer", () => {
  it("keeps the mail's own styles but no title text or scripts", async () => {
    const { HtmlViewer } = await import("@/components/mail/HtmlViewer");
    render(<HtmlViewer html={'<style>.pre{display:none}</style><title>Stray title</title><div class="pre">Preheader</div><p>Hi</p><script>x()</script>'} allowRemote={false} />);
    const doc = screen.getByTestId("email-body").getAttribute("srcdoc")!;
    const body = doc.slice(doc.indexOf("<body>"));
    expect(body).toContain("<style>.pre{display:none}</style>");
    expect(body).not.toContain("Stray title");
    expect(body).not.toContain("<script");
    // The mail's CSS comes after the base look, so it wins.
    expect(doc.indexOf("color-scheme")).toBeLessThan(doc.indexOf(".pre{display:none}"));
  });
});

describe("Sender logos (BIMI)", () => {
  const rowOf = (address: string) =>
    screen.getAllByTestId("email-row").find((r) => {
      const e = useMailStore.getState().emails.find((m) => r.textContent?.includes(m.subject) && m.from.address === address);
      return !!e;
    })!;

  it("shows a verified logo with a check mark by the name, a plain logo without, initials otherwise", async () => {
    render(<EmailList />);
    const github = rowOf("noreply@github.com");
    await waitFor(() => expect(within(github).getByTestId("sender-logo")).toBeInTheDocument());
    expect(within(github).getByTestId("verified-sender")).toHaveTextContent("");

    const stripe = rowOf("receipts@stripe.com");
    await waitFor(() => expect(within(stripe).getByTestId("sender-logo")).toBeInTheDocument());
    expect(within(stripe).queryByTestId("verified-sender")).not.toBeInTheDocument();

    expect(within(rowOf("anna@studio.design")).queryByTestId("sender-logo")).not.toBeInTheDocument();
  });

  it("the reader names who verified the logo", async () => {
    const mail = useMailStore.getState().emails.find((e) => e.from.address === "noreply@github.com")!;
    await useMailStore.getState().selectEmail(mail.id);
    render(<MailReader />);
    const mark = await screen.findByTestId("verified-sender");
    expect(mark).toHaveAttribute("title", expect.stringContaining("DigiCert"));
    expect(mark).toHaveTextContent("");
    // The mark comes before the sender name.
    expect(mark.compareDocumentPosition(screen.getAllByText("GitHub")[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("no lookups and no logos when turned off", async () => {
    useUiStore.getState().setPref("senderLogos", false);
    const spy = vi.spyOn(backend, "senderLogo");
    render(<EmailList />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("sender-logo")).not.toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
    useUiStore.getState().setPref("senderLogos", true);
  });
});
