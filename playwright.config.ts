import { defineConfig, devices } from "@playwright/test";

/**
 * E2E suite. Runs the real React app in Chromium against the in-memory mock
 * backend (the same IPC contract as the Rust core), so it needs no mail
 * servers. See README for running against the packaged Tauri app.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
    viewport: { width: 1400, height: 880 },
    colorScheme: "dark",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1400, height: 880 } } }],
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
