import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MailReader } from "@/components/mail/MailReader";
import { useMailStore } from "@/store/mailStore";
import { splitHtmlQuote, splitTextQuote } from "@/lib/quotes";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

describe("quote folding", () => {
  it("cuts Gmail's quote with its attribution", () => {
    const s = splitHtmlQuote(
      '<div>Thanks!</div><br><div class="gmail_quote"><div class="gmail_attr">On Tue, Support wrote:<br></div><blockquote class="gmail_quote">Hello</blockquote></div>',
    );
    expect(s.quoted).toBe(true);
    expect(s.body).toContain("Thanks!");
    expect(s.body).not.toContain("wrote");
    expect(s.body).not.toContain("Hello");
  });

  it("cuts a trailing blockquote and the line before it", () => {
    const s = splitHtmlQuote("<p>Sure.</p><p>On Mon, 5 Oct 2026, Anna написала:</p><blockquote><p>Can you?</p></blockquote>");
    expect(s).toEqual({ body: "<p>Sure.</p>", quoted: true });
    // No blank lines left where the quote was.
    expect(splitHtmlQuote('<div>Hi</div><br><br><div class="gmail_quote">On X wrote:<blockquote>a</blockquote></div>').body).toBe("<div>Hi</div>");
  });

  it("keeps a message that is nothing but a quote, and one without quotes", () => {
    const onlyQuote = '<div class="gmail_quote">On Tue, Support wrote:<blockquote>Hello it me</blockquote></div>';
    expect(splitHtmlQuote(onlyQuote)).toEqual({ body: onlyQuote, quoted: false });
    expect(splitHtmlQuote("<p>Hi</p><blockquote>a cited passage</blockquote><p>and my take</p>").quoted).toBe(false);
  });

  it("plain text: trailing '>' lines and the attribution", () => {
    expect(splitTextQuote("Sounds good.\n\nOn Tue, Oct 6, 2026, Support\nwrote:\n> Hello\n>\n> Bye\n")).toEqual({ body: "Sounds good.", quoted: true });
    expect(splitTextQuote("> only a quote").quoted).toBe(false);
    expect(splitTextQuote("no quote here").quoted).toBe(false);
  });
});

describe("Conversation view", () => {
  let backend: MockBackend;
  beforeEach(async () => {
    backend = setupBackend({ withDemoAccount: true, seedCount: 6, demoThread: true });
    await useMailStore.getState().init();
  });

  const openLast = async () => {
    const mails = backend.db.emails.filter((e) => e.subject.endsWith("Double charge")).sort((a, b) => a.date - b.date);
    const last = mails[mails.length - 1];
    await useMailStore.getState().selectEmail(last.id);
    render(<MailReader />);
    return mails;
  };

  it("lists every message of the thread, inbox and sent, oldest first", async () => {
    await openLast();
    await waitFor(() => expect(screen.getAllByTestId("thread-message")).toHaveLength(4));
    expect(screen.getByTestId("thread-count")).toHaveTextContent("4 messages");
    const items = screen.getAllByTestId("thread-message");
    expect(items[0]).toHaveTextContent("Alex Morgan");
    expect(items[0]).toHaveTextContent("charged twice");
    expect(items[3]).toHaveAttribute("aria-current", "true");
  });

  it("an earlier message expands to its full text and collapses again", async () => {
    await openLast();
    const toggle = await screen.findByRole("button", { name: "Expand message from Support" });
    const item = toggle.closest("article")!;
    await userEvent.click(toggle);
    await waitFor(() => expect(within(item).getByTestId("email-body")).toBeInTheDocument());
    // Open: the recipients line replaces the snippet, Gmail style.
    expect(within(item).getByRole("button", { name: "Collapse message from Support" })).toHaveAttribute("aria-expanded", "true");
    expect(item).toHaveTextContent("to me");
    await userEvent.click(within(item).getByRole("button", { name: "Collapse message from Support" }));
    expect(within(item).queryByTestId("email-body")).not.toBeInTheDocument();
  });

  it("quotes of earlier messages are folded behind •••", async () => {
    await openLast();
    await waitFor(() => expect(screen.getAllByTestId("thread-message")).toHaveLength(4));
    const current = screen.getAllByTestId("thread-message")[3];
    const body = within(current).getByTestId("email-body") as HTMLIFrameElement;
    expect(body.getAttribute("srcdoc")).toContain("refunded");
    expect(body.getAttribute("srcdoc")).not.toContain("wrote:");
    await userEvent.click(within(current).getByRole("button", { name: "Show quoted text" }));
    expect((within(current).getByTestId("email-body") as HTMLIFrameElement).getAttribute("srcdoc")).toContain("wrote:");
  });

  it("a message on its own looks the same, as a conversation of one, quotes and all", async () => {
    const lone = backend.db.emails.find((e) => !e.subject.endsWith("Double charge") && e.bodyHtml)!;
    lone.bodyHtml = "<p>Hi</p><p>On Mon, Bob wrote:</p><blockquote>earlier</blockquote>";
    await useMailStore.getState().selectEmail(lone.id);
    render(<MailReader />);
    await screen.findByTestId("email-body");
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.getAllByTestId("thread-message")).toHaveLength(1);
    expect(screen.queryByTestId("thread-count")).not.toBeInTheDocument();
    expect(screen.queryByTestId("toggle-quote")).not.toBeInTheDocument();
    expect(screen.getByTestId("email-body").getAttribute("srcdoc")).toContain("earlier");
  });
});
