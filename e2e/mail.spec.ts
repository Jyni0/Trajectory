import { expect, test, type Page } from "@playwright/test";

async function addImapAccount(page: Page, email = "jane@corp.example") {
  await page.getByRole("button", { name: "Add your first account" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByTestId("provider-imap").click();

  const form = page.getByRole("form", { name: "IMAP account" });
  await form.getByLabel("Name", { exact: true }).fill("Jane Doe");
  await form.getByLabel("Email").fill(email);
  await form.getByLabel("Password").click(); // blur email → autodetect
  await expect(form.getByLabel("Incoming host")).toHaveValue("imap.corp.example");
  await expect(form.getByLabel("Outgoing host")).toHaveValue("smtp.corp.example");
  await form.getByLabel("Password").fill("app-password");

  await form.getByRole("button", { name: "Test connection" }).click();
  await expect(page.getByTestId("connection-report")).toContainText("Connected");

  await form.getByRole("button", { name: "Add account" }).click();
  await expect(page.getByRole("dialog")).toBeHidden();
}

test.describe("Trajectory", () => {
  test("onboarding → settings → add account → read mail → save draft", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "No accounts yet" })).toBeVisible();

    await addImapAccount(page);

    // Mail list is populated after the account is created.
    await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
    const rows = page.getByTestId("email-row");
    await expect(rows.first()).toBeVisible();
    expect(await rows.count()).toBeGreaterThan(5);
    await expect(page.getByTestId("account-switcher")).toContainText("jane@corp.example");

    // Open a message in the reader.
    await rows.first().click();
    await expect(page.getByTestId("reader-subject")).toHaveText("Weekly product sync — agenda");
    await expect(page.getByRole("toolbar", { name: "Message actions" })).toBeVisible();

    // Compose a new message and save it as a draft.
    await page.getByRole("button", { name: /New message/ }).first().click();
    const composer = page.getByRole("region", { name: "Composer" });
    await composer.getByRole("textbox", { name: "To" }).fill("friend@example.com");
    await composer.getByRole("textbox", { name: "To" }).press("Enter");
    await composer.getByRole("textbox", { name: "Subject" }).fill("Draft from E2E");
    await composer.getByTestId("composer-editor").click();
    await page.keyboard.type("Hello from Playwright!");
    await composer.getByRole("button", { name: "Save draft" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Draft saved" })).toBeVisible();
    await composer.getByRole("button", { name: "Close composer" }).click();
    await expect(composer).toBeHidden();

    // The draft shows up in Drafts and reopens in the composer.
    await page.getByTestId("folder-drafts").click();
    await expect(page.getByRole("heading", { name: "Drafts" })).toBeVisible();
    const draftRow = page.getByTestId("email-row").filter({ hasText: "Draft from E2E" });
    await expect(draftRow).toHaveCount(1);
    await draftRow.click();
    await expect(page.getByRole("region", { name: "Composer" }).getByRole("textbox", { name: "Subject" })).toHaveValue("Draft from E2E");
    await expect(page.getByTestId("composer-editor")).toContainText("Hello from Playwright!");
  });

  test("filters, search and archive in the demo mailbox", async ({ page }) => {
    await page.goto("/?demo");
    const rows = page.getByTestId("email-row");
    await expect(rows.first()).toBeVisible();

    await page.getByRole("radio", { name: "Unread" }).click();
    await expect(page.getByRole("radio", { name: "Unread" })).toHaveAttribute("aria-checked", "true");
    await expect(page.locator('[data-testid="email-row"]:not([data-unread])')).toHaveCount(0);
    await expect(rows.first()).toHaveAttribute("data-unread", "true");

    await page.getByRole("radio", { name: "All" }).click();
    await page.getByRole("textbox", { name: "Search mail" }).fill("invoice");
    await expect(rows.first()).toContainText("invoice", { ignoreCase: true });

    await page.getByRole("textbox", { name: "Search mail" }).fill("");
    await rows.first().click();
    const subject = await page.getByTestId("reader-subject").textContent();
    await page.getByRole("toolbar", { name: "Message actions" }).getByRole("button", { name: "Archive" }).click();
    await page.getByTestId("folder-archive").click();
    await expect(page.getByTestId("email-row").filter({ hasText: subject! })).toHaveCount(1);
  });

  test("sends a message with the keyboard shortcut", async ({ page }) => {
    await page.goto("/?demo");
    await expect(page.getByTestId("email-row").first()).toBeVisible();
    await page.keyboard.press("c");
    const composer = page.getByRole("region", { name: "Composer" });
    await composer.getByRole("textbox", { name: "To" }).fill("team@example.com,");
    await composer.getByRole("textbox", { name: "Subject" }).fill("Ship it");
    await composer.getByTestId("composer-editor").click();
    await page.keyboard.type("Release is green.");
    await page.keyboard.press("Control+Enter");
    await expect(composer).toBeHidden();
    await page.getByTestId("folder-sent").click();
    await expect(page.getByTestId("email-row").filter({ hasText: "Ship it" })).toHaveCount(1);
  });

  test("rejects wrong credentials before saving the account", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Add your first account" }).click();
    await page.getByTestId("provider-pop3").click();
    const form = page.getByRole("form", { name: "POP3 account" });
    await form.getByLabel("Email").fill("bob@corp.example");
    await form.getByLabel("Password").click();
    await expect(form.getByLabel("Incoming host")).toHaveValue("pop.corp.example");
    await form.getByLabel("Password").fill("wrong");
    await form.getByRole("button", { name: "Add account" }).click();
    await expect(form.getByRole("alert")).toContainText("invalid credentials");
    await expect(page.getByRole("dialog")).toBeVisible();
  });
});
