import { create } from "zustand";
import { api, errorMessage } from "@/lib/api";
import { useUiStore } from "@/store/uiStore";
import type { Account, Email, EmailSummary, Folder, MailFilter, MailSort, SyncState, Tag } from "@/lib/types";

export const PAGE_SIZE = 50;

const SORT_KEY = "trajectory.sort";
const SORTS: MailSort[] = ["newest", "oldest", "unread", "flagged", "sender", "subject"];

function loadSort(): MailSort {
  try {
    const v = localStorage.getItem(SORT_KEY) as MailSort | null;
    return v && SORTS.includes(v) ? v : "newest";
  } catch {
    return "newest";
  }
}

/**
 * Whether the All / Unread / Flagged filter makes sense here: mail you
 * receive (Inbox, server labels, tags) — not Sent, Drafts, Archive, Spam, Trash.
 */
export function hasMailFilters(folder: Folder | null | undefined, tagId: string | null): boolean {
  return !!tagId || folder?.role === "inbox" || folder?.role === "custom";
}

/** Colours offered for new tags (the next unused one is picked). */
export const TAG_COLORS = ["#2563eb", "#16a34a", "#ea580c", "#db2777", "#7c3aed", "#0891b2", "#ca8a04", "#dc2626"];

export interface MailState {
  accounts: Account[];
  activeAccountId: string | null;
  folders: Folder[];
  activeFolderId: string | null;
  emails: EmailSummary[];
  total: number;
  hasMore: boolean;
  filter: MailFilter;
  sort: MailSort;
  search: string;
  /** The user's tags, in their manual order (counts for the active account). */
  tags: Tag[];
  /** Showing every message with this tag instead of a folder. */
  activeTagId: string | null;
  selectedEmailId: string | null;
  activeEmail: Email | null;
  loadingList: boolean;
  /** Per folder: older mail being fetched from the server, or all of it is here. */
  olderState: Record<string, "loading" | "done">;
  loadingEmail: boolean;
  initialized: boolean;
  syncState: Record<string, SyncState>;
  error: string | null;

  init(): Promise<void>;
  reloadAccounts(selectId?: string): Promise<void>;
  selectAccount(id: string): Promise<void>;
  refreshFolders(): Promise<void>;
  selectFolder(id: string): Promise<void>;
  selectFolderByRole(role: Folder["role"]): Promise<void>;
  /** Opens a folder of any account, switching the active account if needed. */
  openFolder(accountId: string, folderId: string): Promise<void>;
  setFilter(filter: MailFilter): Promise<void>;
  setSort(sort: MailSort): Promise<void>;
  loadTags(): Promise<void>;
  selectTag(id: string): Promise<void>;
  createTag(name: string, color?: string): Promise<Tag>;
  updateTag(id: string, name: string, color: string): Promise<void>;
  deleteTag(id: string): Promise<void>;
  /** Moves a tag to `index` in the manual order. */
  moveTag(id: string, index: number): Promise<void>;
  /** Puts a tag on a message, or takes it off (`on` omitted = toggle). */
  tagEmail(emailId: string, tagId: string, on?: boolean): Promise<void>;
  setNote(emailId: string, note: string): Promise<void>;
  setSearch(search: string): Promise<void>;
  loadEmails(): Promise<void>;
  loadMore(): Promise<void>;
  /** At the end of a folder's list: fetches its next older batch from the server. */
  loadOlder(): Promise<void>;
  selectEmail(id: string | null): Promise<void>;
  selectNext(direction: 1 | -1): Promise<void>;
  markRead(id: string, read: boolean): Promise<void>;
  toggleFlag(id: string): Promise<void>;
  archive(id: string): Promise<void>;
  remove(id: string): Promise<void>;
  removeAccount(id: string): Promise<void>;
  setSyncState(accountId: string, state: SyncState): void;
  sync(accountId?: string): Promise<void>;
  activeFolder(): Folder | null;
  activeAccount(): Account | null;
}

// Monotonic token so responses for stale queries are ignored.
let listRequest = 0;

export const useMailStore = create<MailState>((set, get) => {
  const patchSummary = (id: string, patch: Partial<EmailSummary>) =>
    set((s) => ({
      emails: s.emails.map((e) => (e.id === id ? { ...e, ...patch } : e)),
      activeEmail: s.activeEmail?.id === id ? { ...s.activeEmail, ...patch } : s.activeEmail,
    }));

  const adjustUnread = (folderId: string, delta: number) =>
    set((s) => ({
      folders: s.folders.map((f) => (f.id === folderId ? { ...f, unreadCount: Math.max(0, f.unreadCount + delta) } : f)),
    }));

  /** Removes a message from the visible list and selects its neighbour. */
  const dropFromList = (id: string) => {
    const { emails, selectedEmailId } = get();
    const idx = emails.findIndex((e) => e.id === id);
    const rest = emails.filter((e) => e.id !== id);
    set((s) => ({ emails: rest, total: Math.max(0, s.total - 1) }));
    if (selectedEmailId === id) {
      const next = rest[Math.min(idx, rest.length - 1)];
      void get().selectEmail(next?.id ?? null);
    }
  };

  const guard = async (fn: () => Promise<void>) => {
    try {
      await fn();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  };

  return {
    accounts: [],
    activeAccountId: null,
    folders: [],
    activeFolderId: null,
    emails: [],
    total: 0,
    hasMore: false,
    olderState: {},
    filter: "all",
    sort: loadSort(),
    search: "",
    tags: [],
    activeTagId: null,
    selectedEmailId: null,
    activeEmail: null,
    loadingList: false,
    loadingEmail: false,
    initialized: false,
    syncState: {},
    error: null,

    async init() {
      await get().reloadAccounts();
      set({ initialized: true });
    },

    async loadTags() {
      const accountId = get().activeAccountId;
      try {
        const tags = await api.listTags(accountId);
        if (get().activeAccountId === accountId) set({ tags });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    async selectTag(id) {
      set({ activeTagId: id, activeFolderId: null, selectedEmailId: null, activeEmail: null });
      await get().loadEmails();
    },

    async createTag(name, color) {
      const used = new Set(get().tags.map((t) => t.color));
      const tag = await api.createTag(name, color ?? TAG_COLORS.find((c) => !used.has(c)) ?? TAG_COLORS[get().tags.length % TAG_COLORS.length]);
      set((s) => ({ tags: [...s.tags, tag] }));
      return tag;
    },

    async updateTag(id, name, color) {
      const tag = await api.updateTag(id, name, color);
      set((s) => ({ tags: s.tags.map((t) => (t.id === id ? { ...tag, count: t.count } : t)) }));
    },

    async deleteTag(id) {
      await api.deleteTag(id);
      const strip = (e: EmailSummary) => (e.tags.includes(id) ? { ...e, tags: e.tags.filter((t) => t !== id) } : e);
      set((s) => ({
        tags: s.tags.filter((t) => t.id !== id),
        emails: s.emails.map(strip),
        activeEmail: s.activeEmail && (strip(s.activeEmail) as Email),
      }));
      if (get().activeTagId === id) {
        set({ activeTagId: null });
        const inbox = get().folders.find((f) => f.role === "inbox");
        if (inbox) await get().selectFolder(inbox.id);
      }
    },

    async moveTag(id, index) {
      const tags = [...get().tags];
      const from = tags.findIndex((t) => t.id === id);
      if (from < 0) return;
      const [tag] = tags.splice(from, 1);
      tags.splice(Math.max(0, Math.min(index, tags.length)), 0, tag);
      if (tags.every((t, i) => t.id === get().tags[i]?.id)) return;
      set({ tags: tags.map((t, i) => ({ ...t, position: i })) });
      await guard(() => api.reorderTags(tags.map((t) => t.id)));
    },

    async tagEmail(emailId, tagId, on) {
      const mail = get().emails.find((e) => e.id === emailId) ?? (get().activeEmail?.id === emailId ? get().activeEmail : null);
      const has = mail ? mail.tags.includes(tagId) : false;
      const next = on ?? !has;
      if (mail && next === has) return;
      if (mail) patchSummary(emailId, { tags: next ? [...mail.tags, tagId] : mail.tags.filter((t) => t !== tagId) });
      set((s) => ({ tags: s.tags.map((t) => (t.id === tagId ? { ...t, count: Math.max(0, t.count + (next ? 1 : -1)) } : t)) }));
      if (!next && get().activeTagId === tagId) dropFromList(emailId);
      await guard(() => api.setEmailTag(emailId, tagId, next));
    },

    async setNote(emailId, note) {
      const clean = note.trim();
      patchSummary(emailId, { hasNote: !!clean });
      set((s) => ({ activeEmail: s.activeEmail?.id === emailId ? { ...s.activeEmail, note: clean || null } : s.activeEmail }));
      await guard(() => api.setNote(emailId, clean || null));
    },

    async reloadAccounts(selectId) {
      const accounts = await api.listAccounts();
      set({ accounts });
      const current = get().activeAccountId;
      const target = selectId ?? (accounts.some((a) => a.id === current) ? current : accounts[0]?.id);
      if (target && target !== current) await get().selectAccount(target);
      if (!target) set({ activeAccountId: null, folders: [], activeFolderId: null, activeTagId: null, emails: [], total: 0, activeEmail: null, selectedEmailId: null });
    },

    async selectAccount(id) {
      set({ activeAccountId: id, activeFolderId: null, activeTagId: null, selectedEmailId: null, activeEmail: null, emails: [], search: "" });
      const folders = await api.listFolders(id);
      set({ folders });
      void get().loadTags();
      const inbox = folders.find((f) => f.role === "inbox") ?? folders[0];
      if (inbox) await get().selectFolder(inbox.id);
    },

    async refreshFolders() {
      const id = get().activeAccountId;
      if (!id) return;
      const folders = await api.listFolders(id);
      if (get().activeAccountId === id) set({ folders });
    },

    async selectFolder(id) {
      const folder = get().folders.find((f) => f.id === id);
      // A filter hidden in this folder must not keep narrowing its list.
      const filter = hasMailFilters(folder, null) ? get().filter : "all";
      set({ activeFolderId: id, activeTagId: null, selectedEmailId: null, activeEmail: null, filter });
      // Show what is cached right away; new mail arrives via `mail:changed`.
      api.syncFolder(id).catch(() => {});
      await get().loadEmails();
    },

    async selectFolderByRole(role) {
      const folder = get().folders.find((f) => f.role === role);
      if (folder) await get().selectFolder(folder.id);
    },

    async openFolder(accountId, folderId) {
      if (get().activeAccountId !== accountId) {
        set({ activeAccountId: accountId, activeFolderId: null, selectedEmailId: null, activeEmail: null, emails: [], search: "" });
        const folders = await api.listFolders(accountId);
        set({ folders });
        void get().loadTags();
      }
      await get().selectFolder(folderId);
    },

    async setFilter(filter) {
      set({ filter });
      await get().loadEmails();
    },

    async setSort(sort) {
      set({ sort });
      try {
        localStorage.setItem(SORT_KEY, sort);
      } catch {
        /* storage unavailable */
      }
      await get().loadEmails();
    },

    async setSearch(search) {
      set({ search });
      await get().loadEmails();
    },

    async loadEmails() {
      const { activeAccountId, activeFolderId, activeTagId, filter, sort, search } = get();
      if (!activeAccountId) return;
      const token = ++listRequest;
      set({ loadingList: true });
      try {
        const page = await api.listEmails({
          accountId: activeAccountId,
          folderId: activeTagId ? null : activeFolderId,
          tagId: activeTagId,
          filter,
          sort,
          search: search.trim() || null,
          limit: PAGE_SIZE,
          offset: 0,
        });
        if (token !== listRequest) return;
        set({ emails: page.items, total: page.total, hasMore: page.hasMore, error: null });
      } catch (e) {
        if (token === listRequest) set({ error: errorMessage(e) });
      } finally {
        if (token === listRequest) set({ loadingList: false });
      }
    },

    async loadMore() {
      const { hasMore, loadingList, activeAccountId, activeFolderId, activeTagId, filter, sort, search, emails } = get();
      if (!hasMore || loadingList || !activeAccountId) return;
      const token = ++listRequest;
      set({ loadingList: true });
      try {
        const page = await api.listEmails({
          accountId: activeAccountId,
          folderId: activeTagId ? null : activeFolderId,
          tagId: activeTagId,
          filter,
          sort,
          search: search.trim() || null,
          limit: PAGE_SIZE,
          offset: emails.length,
        });
        if (token !== listRequest) return;
        const seen = new Set(emails.map((e) => e.id));
        set((s) => ({ emails: [...s.emails, ...page.items.filter((e) => !seen.has(e.id))], total: page.total, hasMore: page.hasMore }));
      } finally {
        if (token === listRequest) set({ loadingList: false });
      }
    },

    async loadOlder() {
      const { activeFolderId: folderId, activeTagId, search, olderState, hasMore, loadingList } = get();
      // Only a plain folder view pages back on the server.
      if (!folderId || activeTagId || search.trim() || hasMore || loadingList || olderState[folderId]) return;
      set((s) => ({ olderState: { ...s.olderState, [folderId]: "loading" } }));
      let done = false;
      try {
        const page = await api.loadOlder(folderId);
        done = !page.hasMore;
        if (page.added > 0 && get().activeFolderId === folderId) {
          set({ hasMore: true });
          await get().loadMore();
        }
      } catch (e) {
        // Offline or rate limited: try again on the next scroll to the end.
        useUiStore.getState().pushToast(errorMessage(e), "error");
      } finally {
        set((s) => {
          const next = { ...s.olderState };
          if (done) next[folderId] = "done";
          else delete next[folderId];
          return { olderState: next };
        });
      }
    },

    async selectEmail(id) {
      // Reading replaces the composer: what was written goes to Drafts first.
      if (id && !(await useUiStore.getState().leaveComposer())) return;
      set({ selectedEmailId: id, activeEmail: id ? get().activeEmail : null });
      if (!id) return;
      set({ loadingEmail: true });
      try {
        const email = await api.getEmail(id);
        if (get().selectedEmailId !== id) return;
        set({ activeEmail: email });
        if (!email.isRead) await get().markRead(id, true);
      } catch (e) {
        set({ error: errorMessage(e) });
      } finally {
        if (get().selectedEmailId === id) set({ loadingEmail: false });
      }
    },

    async selectNext(direction) {
      const { emails, selectedEmailId } = get();
      if (emails.length === 0) return;
      const idx = emails.findIndex((e) => e.id === selectedEmailId);
      const next = idx < 0 ? emails[0] : emails[Math.min(emails.length - 1, Math.max(0, idx + direction))];
      if (next && next.id !== selectedEmailId) await get().selectEmail(next.id);
    },

    async markRead(id, read) {
      const mail = get().emails.find((e) => e.id === id) ?? get().activeEmail;
      if (mail && mail.isRead !== read) adjustUnread(mail.folderId, read ? -1 : 1);
      patchSummary(id, { isRead: read });
      await guard(() => api.markRead(id, read));
    },

    async toggleFlag(id) {
      const mail = get().emails.find((e) => e.id === id) ?? get().activeEmail;
      if (!mail) return;
      const flagged = !mail.isFlagged;
      patchSummary(id, { isFlagged: flagged });
      if (!flagged && get().filter === "flagged") dropFromList(id);
      await guard(() => api.setFlagged(id, flagged));
    },

    async archive(id) {
      dropFromList(id);
      await guard(() => api.archiveEmail(id));
      await get().refreshFolders();
    },

    async remove(id) {
      dropFromList(id);
      await guard(() => api.deleteEmail(id));
      await get().refreshFolders();
    },

    async removeAccount(id) {
      await api.removeAccount(id);
      if (get().activeAccountId === id) set({ activeAccountId: null });
      await get().reloadAccounts();
    },

    setSyncState(accountId, state) {
      set((s) => ({ syncState: { ...s.syncState, [accountId]: state } }));
    },

    async sync(accountId) {
      await api.syncNow(accountId ?? get().activeAccountId ?? undefined);
    },

    activeFolder() {
      const { folders, activeFolderId } = get();
      return folders.find((f) => f.id === activeFolderId) ?? null;
    },

    activeAccount() {
      const { accounts, activeAccountId } = get();
      return accounts.find((a) => a.id === activeAccountId) ?? null;
    },
  };
});
