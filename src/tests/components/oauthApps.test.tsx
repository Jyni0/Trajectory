import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OAuthAppsPanel } from "@/components/settings/OAuthAppsPanel";
import type { MockBackend } from "@/lib/mockBackend";
import { setupBackend } from "../utils";

const GOOGLE_ID = "123456789-abc.apps.googleusercontent.com";

describe("Settings → Sign-in apps", () => {
  let backend: MockBackend;
  beforeEach(() => {
    backend = setupBackend({ withDemoAccount: false });
  });

  it("saves a Google app once; the secret is never shown again", async () => {
    render(<OAuthAppsPanel />);
    const google = await screen.findByRole("form", { name: "Google sign-in app" });
    const card = google.closest("div.rounded-2xl") as HTMLElement;
    expect(within(card).getByText("Not set up")).toBeInTheDocument();

    await userEvent.type(within(google).getByLabelText("Google client ID"), GOOGLE_ID);
    await userEvent.type(within(google).getByLabelText("Google client secret"), "GOCSPX-supersecret");
    await userEvent.click(within(google).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(within(card).getByText("Saved in keychain")).toBeInTheDocument());
    const secret = within(google).getByLabelText("Google client secret") as HTMLInputElement;
    expect(secret.value).toBe("");
    expect(secret.placeholder).toBe("GOCSPX-••••••");
    expect(document.body.textContent).not.toContain("supersecret");

    const apps = await backend.oauthApps();
    expect(apps.find((a) => a.provider === "google")).toMatchObject({ clientId: GOOGLE_ID, source: "settings" });
  });

  it("explains a wrong client ID and a missing Google secret", async () => {
    render(<OAuthAppsPanel />);
    const google = await screen.findByRole("form", { name: "Google sign-in app" });
    await userEvent.type(within(google).getByLabelText("Google client ID"), "oops");
    await userEvent.click(within(google).getByRole("button", { name: "Save" }));
    expect(await within(google).findByText(/ends with \.apps\.googleusercontent\.com/)).toBeInTheDocument();

    await userEvent.clear(within(google).getByLabelText("Google client ID"));
    await userEvent.type(within(google).getByLabelText("Google client ID"), GOOGLE_ID);
    await userEvent.click(within(google).getByRole("button", { name: "Save" }));
    expect(await within(google).findByText(/needs its client secret/)).toBeInTheDocument();
  });

  it("Microsoft needs only the client ID, and can be forgotten", async () => {
    render(<OAuthAppsPanel />);
    const ms = await screen.findByRole("form", { name: "Microsoft sign-in app" });
    expect(within(ms).queryByLabelText("Microsoft client secret")).not.toBeInTheDocument();
    await userEvent.type(within(ms).getByLabelText("Microsoft client ID"), "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0");
    await userEvent.click(within(ms).getByRole("button", { name: "Save" }));
    await userEvent.click(await within(ms).findByRole("button", { name: "Forget" }));
    await userEvent.click(within(ms).getByRole("button", { name: "Forget" }));
    await waitFor(async () => expect((await backend.oauthApps()).find((a) => a.provider === "microsoft")?.source).toBeNull());
  });
});
