import { beforeEach, describe, expect, it, vi } from "vitest";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { emptyDraft } from "@/lib/compose";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

let backend: MockBackend;
const store = () => useMailStore.getState();

beforeEach(async () => {
  backend = setupBackend();
  await store().init();
});

describe("mailStore — accounts & folders", () => {
  it("init selects the first account and its inbox", () => {
    const s = store();
    expect(s.initialized).toBe(true);
    expect(s.accounts).toHaveLength(1);
    expect(s.activeAccountId).toBe(s.accounts[0].id);
    expect(s.activeFolder()?.role).toBe("inbox");
    expect(s.emails.length).toBe(12);
    expect(s.emails.every((e) => e.folderId === s.activeFolderId)).toBe(true);
  });

  it("switching folders loads that folder's mail and clears the selection", async () => {
    await store().selectEmail(store().emails[0].id);
    await store().selectFolderByRole("sent");
    const s = store();
    expect(s.activeFolder()?.role).toBe("sent");
    expect(s.selectedEmailId).toBeNull();
    expect(s.emails.length).toBe(4);
    expect(s.emails.every((e) => e.folderId === s.activeFolderId)).toBe(true);
  });

  it("switching accounts reloads folders", async () => {
    const second = await backend.addOAuthAccount("gmail");
    await store().reloadAccounts(second.id);
    expect(store().activeAccountId).toBe(second.id);
    expect(store().folders.every((f) => f.accountId === second.id)).toBe(true);
  });

  it("removing the active account falls back to another one", async () => {
    const second = await backend.addOAuthAccount("outlook");
    await store().reloadAccounts(second.id);
    await store().removeAccount(second.id);
    expect(store().accounts).toHaveLength(1);
    expect(store().activeAccountId).toBe(store().accounts[0].id);
  });

  it("removing the last account clears everything", async () => {
    await store().removeAccount(store().accounts[0].id);
    const s = store();
    expect(s.activeAccountId).toBeNull();
    expect(s.emails).toEqual([]);
    expect(s.folders).toEqual([]);
  });
});

describe("mailStore — filtering & search", () => {
  it("filters unread and flagged messages", async () => {
    await store().setFilter("unread");
    expect(store().emails.length).toBeGreaterThan(0);
    expect(store().emails.every((e) => !e.isRead)).toBe(true);

    await store().setFilter("flagged");
    expect(store().emails.length).toBeGreaterThan(0);
    expect(store().emails.every((e) => e.isFlagged)).toBe(true);

    await store().setFilter("all");
    expect(store().emails.length).toBe(12);
  });

  it("searches subject, sender and body", async () => {
    await store().setSearch("invoice");
    expect(store().emails.length).toBeGreaterThan(0);
    expect(store().emails.every((e) => /invoice/i.test(e.subject + e.snippet))).toBe(true);

    await store().setSearch("github.com");
    expect(store().emails.every((e) => e.from.address === "noreply@github.com")).toBe(true);

    await store().setSearch("no-such-term-anywhere");
    expect(store().emails).toEqual([]);
    expect(store().total).toBe(0);
  });

  it("ignores responses from superseded queries", async () => {
    const slow = backend.listEmails.bind(backend);
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const spy = vi.spyOn(backend, "listEmails").mockImplementationOnce(async (q) => {
      await gate;
      return slow(q);
    });
    const first = store().setFilter("flagged");
    await store().setFilter("unread");
    release();
    await first;
    expect(store().filter).toBe("unread");
    expect(store().emails.every((e) => !e.isRead)).toBe(true);
    spy.mockRestore();
  });

  it("paginates with loadMore", async () => {
    backend = setupBackend({ withDemoAccount: true, seedCount: 120 });
    await store().init();
    expect(store().emails).toHaveLength(50);
    expect(store().hasMore).toBe(true);
    await store().loadMore();
    await store().loadMore();
    expect(store().emails).toHaveLength(120);
    expect(store().hasMore).toBe(false);
    expect(new Set(store().emails.map((e) => e.id)).size).toBe(120);
  });
});

describe("mailStore — message actions", () => {
  it("opening an unread email marks it read and decrements the folder badge", async () => {
    const unread = store().emails.find((e) => !e.isRead)!;
    const before = store().activeFolder()!.unreadCount;
    await store().selectEmail(unread.id);
    expect(store().activeEmail?.id).toBe(unread.id);
    expect(store().emails.find((e) => e.id === unread.id)?.isRead).toBe(true);
    expect(store().activeFolder()!.unreadCount).toBe(before - 1);
    expect(backend.db.emails.find((e) => e.id === unread.id)?.isRead).toBe(true);
  });

  it("toggles flags optimistically and persists them", async () => {
    const id = store().emails.find((e) => !e.isFlagged)!.id;
    await store().toggleFlag(id);
    expect(store().emails.find((e) => e.id === id)?.isFlagged).toBe(true);
    expect(backend.db.emails.find((e) => e.id === id)?.isFlagged).toBe(true);
  });

  it("unflagging inside the Flagged filter removes the row", async () => {
    await store().setFilter("flagged");
    const id = store().emails[0].id;
    await store().toggleFlag(id);
    expect(store().emails.some((e) => e.id === id)).toBe(false);
  });

  it("archive removes the message and selects the next one", async () => {
    const [first, second] = store().emails;
    await store().selectEmail(first.id);
    await store().archive(first.id);
    expect(store().emails.some((e) => e.id === first.id)).toBe(false);
    expect(store().selectedEmailId).toBe(second.id);
    const archive = store().folders.find((f) => f.role === "archive")!;
    expect(backend.db.emails.find((e) => e.id === first.id)?.folderId).toBe(archive.id);
  });

  it("delete moves to Trash, deleting from Trash is permanent", async () => {
    const id = store().emails[0].id;
    await store().remove(id);
    await store().selectFolderByRole("trash");
    expect(store().emails.map((e) => e.id)).toContain(id);
    await store().remove(id);
    expect(backend.db.emails.some((e) => e.id === id)).toBe(false);
  });

  it("j/k style navigation walks the list", async () => {
    await store().selectNext(1);
    expect(store().selectedEmailId).toBe(store().emails[0].id);
    await store().selectNext(1);
    expect(store().selectedEmailId).toBe(store().emails[1].id);
    await store().selectNext(-1);
    expect(store().selectedEmailId).toBe(store().emails[0].id);
  });

  it("tracks sync state per account", () => {
    store().setSyncState("a", "syncing");
    store().setSyncState("b", "error");
    expect(store().syncState).toEqual({ a: "syncing", b: "error" });
  });
});

describe("uiStore", () => {
  it("opens and closes the composer and settings", () => {
    const ui = useUiStore.getState();
    ui.openComposer(emptyDraft("acc"));
    expect(useUiStore.getState().composer?.accountId).toBe("acc");
    ui.closeComposer();
    expect(useUiStore.getState().composer).toBeNull();

    ui.openSettings("add");
    expect(useUiStore.getState()).toMatchObject({ settingsOpen: true, settingsTab: "add" });
    ui.closeSettings();
    expect(useUiStore.getState().settingsOpen).toBe(false);
  });

  it("auto-dismisses toasts and caps the stack", () => {
    vi.useFakeTimers();
    const ui = useUiStore.getState();
    for (let i = 0; i < 6; i++) ui.pushToast(`t${i}`);
    expect(useUiStore.getState().toasts.map((t) => t.message)).toEqual(["t2", "t3", "t4", "t5"]);
    vi.advanceTimersByTime(4000);
    expect(useUiStore.getState().toasts).toEqual([]);
    vi.useRealTimers();
  });

  it("persists preferences", () => {
    useUiStore.getState().setPref("loadRemoteImages", true);
    expect(useUiStore.getState().prefs.loadRemoteImages).toBe(true);
    expect(JSON.parse(localStorage.getItem("trajectory.prefs")!)).toMatchObject({ loadRemoteImages: true });
  });
});
