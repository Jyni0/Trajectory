import { beforeEach, describe, expect, it } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { EmailList } from "@/components/mail/EmailList";
import { MailReader } from "@/components/mail/MailReader";
import { Sidebar } from "@/components/layout/Sidebar";
import { useMailStore } from "@/store/mailStore";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

let backend: MockBackend;
const store = () => useMailStore.getState();

beforeEach(async () => {
  backend = setupBackend();
  await store().init();
});

/** Minimal DataTransfer for HTML5 drag & drop in jsdom. */
function dataTransfer() {
  const data: Record<string, string> = {};
  return {
    data,
    setData: (k: string, v: string) => void (data[k] = v),
    getData: (k: string) => data[k] ?? "",
    get types() {
      return Object.keys(data);
    },
    dropEffect: "none",
    effectAllowed: "all",
  };
}

describe("mailStore — tags, notes, sort", () => {
  it("loads the demo tags with per-account counts", () => {
    expect(store().tags.map((t) => [t.name, t.count])).toEqual([
      ["Follow up", 1],
      ["Work", 2],
    ]);
  });

  it("creates a tag with the next free colour and toggles it on a message", async () => {
    const tag = await store().createTag("Invoices");
    expect(tag.color).not.toBe(store().tags[0].color);
    const id = store().emails[3].id;
    await store().tagEmail(id, tag.id);
    expect(store().emails[3].tags).toContain(tag.id);
    expect(store().tags.find((t) => t.id === tag.id)?.count).toBe(1);
    expect(backend.db.emails.find((e) => e.id === id)?.tags).toContain(tag.id);
    await store().tagEmail(id, tag.id);
    expect(store().emails[3].tags).not.toContain(tag.id);
    expect(store().tags.find((t) => t.id === tag.id)?.count).toBe(0);
  });

  it("rejects duplicate and blank names", async () => {
    await expect(store().createTag("work")).rejects.toMatchObject({ kind: "invalid" });
    await expect(store().createTag("   ")).rejects.toMatchObject({ kind: "invalid" });
  });

  it("a tag view lists only tagged mail; untagging drops the row", async () => {
    const work = store().tags.find((t) => t.name === "Work")!;
    await store().selectTag(work.id);
    expect(store().activeTagId).toBe(work.id);
    expect(store().emails).toHaveLength(2);
    await store().tagEmail(store().emails[0].id, work.id, false);
    expect(store().emails).toHaveLength(1);
    await store().selectFolderByRole("inbox");
    expect(store().activeTagId).toBeNull();
  });

  it("reorders, renames and deletes tags", async () => {
    const [follow, work] = store().tags;
    await store().moveTag(work.id, 0);
    expect(store().tags.map((t) => t.name)).toEqual(["Work", "Follow up"]);
    expect((await backend.listTags()).map((t) => t.name)).toEqual(["Work", "Follow up"]);

    await store().updateTag(follow.id, "Later", "#dc2626");
    expect(store().tags[1]).toMatchObject({ name: "Later", color: "#dc2626", count: 1 });

    await store().selectTag(work.id);
    await store().deleteTag(work.id);
    expect(store().tags.map((t) => t.name)).toEqual(["Later"]);
    expect(store().activeTagId).toBeNull();
    expect(store().activeFolder()?.role).toBe("inbox");
    expect(store().emails.every((e) => !e.tags.includes(work.id))).toBe(true);
  });

  it("saves, finds and clears a note", async () => {
    const id = store().emails[5].id;
    await store().selectEmail(id);
    await store().setNote(id, "  call the accountant ");
    expect(store().activeEmail?.note).toBe("call the accountant");
    expect(store().emails[5].hasNote).toBe(true);
    await store().setSearch("accountant");
    expect(store().emails.map((e) => e.id)).toEqual([id]);
    await store().setNote(id, " ");
    expect(backend.db.emails.find((e) => e.id === id)?.note).toBeNull();
  });

  it("sorts the list and remembers the choice", async () => {
    await store().setSort("oldest");
    const dates = store().emails.map((e) => e.date);
    expect(dates).toEqual([...dates].sort((a, b) => a - b));
    expect(localStorage.getItem("trajectory.sort")).toBe("oldest");

    await store().setSort("unread");
    const firstRead = store().emails.findIndex((e) => e.isRead);
    expect(store().emails.slice(firstRead).every((e) => e.isRead)).toBe(true);

    await store().setSort("subject");
    const subjects = store().emails.map((e) => e.subject.toLowerCase());
    expect(subjects).toEqual([...subjects].sort((a, b) => a.localeCompare(b)));
    await store().setSort("newest");
  });
});

describe("Tags UI", () => {
  it("rows show tag chips and the note mark", () => {
    render(<EmailList />);
    const first = screen.getAllByTestId("email-row")[0];
    expect(within(first).getAllByTestId("tag-chip").map((c) => c.textContent)).toEqual(["Follow up", "Work"]);
    expect(within(first).getByLabelText("Has a note")).toBeInTheDocument();
  });

  it("the reader's tag picker toggles tags and creates new ones", async () => {
    const id = store().emails[1].id;
    await store().selectEmail(id);
    render(<MailReader />);
    await userEvent.click(screen.getByRole("button", { name: "Tags (T)" }));
    const picker = await screen.findByRole("dialog", { name: "Tags" });
    await userEvent.click(within(picker).getByRole("option", { name: /Work/ }));
    await waitFor(() => expect(store().activeEmail?.tags).toHaveLength(1));
    expect(within(picker).getByRole("option", { name: /Work/ })).toHaveAttribute("aria-selected", "true");

    await userEvent.type(within(picker).getByLabelText("Find or create a tag"), "Receipts{Enter}");
    await waitFor(() => expect(store().tags.map((t) => t.name)).toContain("Receipts"));
    expect(store().activeEmail?.tags).toHaveLength(2);
    expect(screen.getAllByTestId("tag-chip").map((c) => c.textContent)).toEqual(["Work", "Receipts"]);

    await userEvent.click(screen.getByRole("button", { name: "Remove tag Work" }));
    expect(store().activeEmail?.tags).toHaveLength(1);
  });

  it("writes a note from the reader", async () => {
    const id = store().emails[2].id;
    await store().selectEmail(id);
    render(<MailReader />);
    await userEvent.click(screen.getByRole("button", { name: "Add note" }));
    const note = screen.getByRole("textbox", { name: "Note" });
    await userEvent.type(note, "Reply after the release");
    act(() => note.blur());
    await waitFor(() => expect(backend.db.emails.find((e) => e.id === id)?.note).toBe("Reply after the release"));
    expect(store().emails[2].hasNote).toBe(true);
  });

  it("keys 1–9 toggle tags on the selected message", async () => {
    render(<App />);
    await screen.findAllByTestId("email-row");
    await userEvent.keyboard("j"); // select the first message
    await waitFor(() => expect(store().selectedEmailId).not.toBeNull());
    const id = store().selectedEmailId!;
    const before = store().emails.find((e) => e.id === id)!.tags.length;
    await userEvent.keyboard("2");
    expect(store().emails.find((e) => e.id === id)!.tags.length).not.toBe(before);
  });

  it("right-click menu tags a message", async () => {
    render(<EmailList />);
    const row = screen.getAllByTestId("email-row")[4];
    fireEvent.contextMenu(row, { clientX: 100, clientY: 100 });
    await userEvent.click(await screen.findByRole("button", { name: /Follow up/ }));
    expect(store().emails[4].tags).toContain(store().tags[0].id);
  });

  it("dropping a message on a sidebar tag tags it; dragging a tag reorders", async () => {
    render(
      <>
        <Sidebar />
        <EmailList />
      </>,
    );
    const row = screen.getAllByTestId("email-row")[6];
    const dt = dataTransfer();
    fireEvent.dragStart(row, { dataTransfer: dt });
    const [followRow, workRow] = screen.getAllByTestId("tag-row");
    fireEvent.dragOver(workRow, { dataTransfer: dt });
    fireEvent.drop(workRow, { dataTransfer: dt });
    await waitFor(() => expect(store().emails[6].tags).toContain(store().tags[1].id));

    const dt2 = dataTransfer();
    fireEvent.dragStart(followRow, { dataTransfer: dt2 });
    // jsdom rects are all zero, so the pointer counts as the lower half: after "Work".
    fireEvent.dragOver(workRow, { dataTransfer: dt2 });
    fireEvent.drop(workRow, { dataTransfer: dt2 });
    await waitFor(() => expect(store().tags.map((t) => t.name)).toEqual(["Work", "Follow up"]));
  });

  it("the account dropdown switches mailboxes", async () => {
    const second = await backend.addOAuthAccount("gmail");
    await store().reloadAccounts();
    render(<Sidebar />);
    await userEvent.click(screen.getByTestId("account-switcher"));
    const options = await screen.findAllByTestId("account-option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    await userEvent.click(options[1]);
    await waitFor(() => expect(store().activeAccountId).toBe(second.id));
    expect(screen.getByTestId("account-switcher")).toHaveAccessibleName(`Account: ${second.email}`);
  });

  it("accounts show their domain's BIMI logo and whether it is verified", async () => {
    await backend.addOAuthAccount("gmail");
    await store().reloadAccounts();
    render(<Sidebar />);
    await userEvent.click(screen.getByTestId("account-switcher"));
    const [own, gmail] = await screen.findAllByTestId("account-option");
    await waitFor(() => expect(within(gmail).getByTestId("sender-logo")).toBeInTheDocument());
    expect(within(gmail).getByTestId("verified-sender")).toBeInTheDocument();
    expect(within(own).queryByTestId("sender-logo")).not.toBeInTheDocument();
    expect(within(own).queryByTestId("verified-sender")).not.toBeInTheDocument();
  });
});

describe("account security", () => {
  it("refuses plaintext connections to public servers", async () => {
    const account = {
      kind: "imap" as const,
      displayName: "Me",
      email: "me@corp.example",
      incoming: { host: "imap.corp.example", port: 143, security: "none" as const },
      outgoing: { host: "smtp.corp.example", port: 465, security: "tls" as const },
      username: null,
      password: "pw",
      pop3DownloadDays: null,
    };
    await expect(backend.testConnection(account)).rejects.toMatchObject({ kind: "invalid" });
    await expect(backend.testConnection({ ...account, incoming: { ...account.incoming, host: "192.168.1.10" } })).resolves.toMatchObject({ incomingOk: true });
  });
});
