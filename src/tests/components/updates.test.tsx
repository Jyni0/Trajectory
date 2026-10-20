import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateRow } from "@/components/settings/UpdateRow";
import { selectHasUpdate, useUpdateStore, type UpdateInfo } from "@/store/updateStore";

const RELEASE: UpdateInfo = {
  version: "2.9.0",
  notes: "Faster sync",
  pageUrl: "https://github.com/Jyni0/Trajectory/releases/tag/Release-2.9.0",
  assetName: "Trajectory_2.9.0_x64-setup.exe",
};

beforeEach(() => {
  useUpdateStore.setState({ status: "idle", release: null, progress: null, error: null });
});

describe("Updates (Settings → About)", () => {
  it("offers to check while nothing newer is known", () => {
    render(<UpdateRow />);
    expect(screen.getByRole("button", { name: /check for updates/i })).toBeInTheDocument();
    expect(selectHasUpdate(useUpdateStore.getState())).toBe(false);
  });

  it("installs a newer release in place and shows its notes", async () => {
    const install = vi.fn(async () => {});
    useUpdateStore.setState({ status: "available", release: RELEASE, install });
    expect(selectHasUpdate(useUpdateStore.getState())).toBe(true);
    render(<UpdateRow />);
    expect(screen.getByText("Faster sync")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Update to 2.9.0" }));
    expect(install).toHaveBeenCalledTimes(1);
  });

  it("shows progress while downloading and blocks a second install", () => {
    useUpdateStore.setState({ status: "downloading", release: RELEASE, progress: 0.4 });
    render(<UpdateRow />);
    expect(screen.getByTestId("install-update")).toBeDisabled();
  });

  it("falls back to the release page only when it has no build for this system", () => {
    useUpdateStore.setState({ status: "available", release: { ...RELEASE, assetName: null } });
    render(<UpdateRow />);
    expect(screen.getByRole("button", { name: "Open release 2.9.0" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /update to/i })).not.toBeInTheDocument();
  });

  it("explains a failed install and lets it be retried", () => {
    useUpdateStore.setState({ status: "error", release: RELEASE, error: "network error: timed out" });
    render(<UpdateRow />);
    expect(screen.getByRole("alert")).toHaveTextContent("Update failed: network error: timed out");
    expect(screen.getByRole("button", { name: "Update to 2.9.0" })).toBeEnabled();
  });
});
