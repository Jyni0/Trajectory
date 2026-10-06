import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { EmailList } from "@/components/mail/EmailList";
import { useMailStore } from "@/store/mailStore";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

describe("Older mail on demand", () => {
  let backend: MockBackend;
  beforeEach(async () => {
    backend = setupBackend({ withDemoAccount: true, seedCount: 3 });
    await useMailStore.getState().init();
  });

  const inboxId = () => useMailStore.getState().activeFolderId!;

  it("fetches the next older batch only once the end of the list is reached", async () => {
    const template = backend.db.emails.find((e) => e.folderId === inboxId())!;
    backend.older.set(
      inboxId(),
      Array.from({ length: 60 }, (_, i) => ({ ...template, id: `old-${i}`, subject: `Old ${i}`, date: template.date - 86400 * (i + 1), tags: [] })),
    );
    const spy = vi.spyOn(backend, "loadOlder");
    render(<EmailList />);

    // The three stored messages fit on screen: the end is reached, one batch of 50 comes.
    await waitFor(() => expect(useMailStore.getState().emails.length).toBe(53));
    expect(spy).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Old 0")).toBeInTheDocument();
    // Ten are still on the "server": nothing more until the user scrolls there.
    await new Promise((r) => setTimeout(r, 30));
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("stops asking once the server has nothing older", async () => {
    const spy = vi.spyOn(backend, "loadOlder");
    render(<EmailList />);
    await waitFor(() => expect(useMailStore.getState().olderState[inboxId()]).toBe("done"));
    await new Promise((r) => setTimeout(r, 30));
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("is not used for tag views or searches", async () => {
    const spy = vi.spyOn(backend, "loadOlder");
    await useMailStore.getState().setSearch("zzz-nothing");
    render(<EmailList />);
    await new Promise((r) => setTimeout(r, 30));
    expect(spy).not.toHaveBeenCalled();
  });
});
