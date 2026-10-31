import { defineConfig, devices } from "@playwright/test";

// End-to-end tests drive the real UI in Chromium against the Vite dev server, with the Tauri bridge
// replaced by an in-page mock (see e2e/bridge.ts). They check what a user sees and does; the Rust side
// has its own tests.
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  fullyParallel: true,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:1420",
    viewport: { width: 1100, height: 720 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1100, height: 720 } } }],
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
