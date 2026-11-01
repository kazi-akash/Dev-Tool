import { expect, test, type Page } from "@playwright/test";
import { calls, installBridge } from "./bridge";

const GB = 1024 ** 3;

/** Fail a test on uncaught page errors and console errors, so broken pages cannot pass quietly. */
function watchErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(`console: ${m.text()}`);
  });
  return errors;
}

test("dashboard answers: healthy? what? why? what can I do?", async ({ page }) => {
  const errors = watchErrors(page);
  await installBridge(page);
  await page.goto("/");

  await expect(page.getByText("Problem detected")).toBeVisible();
  await expect(page.getByText(/primary contributors/)).toBeVisible();
  await expect(page.getByText("Docker / WSL2").first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Analyze" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Optimize" })).toBeVisible();
  await expect(page.getByRole("progressbar", { name: "RAM" })).toHaveAttribute("aria-valuenow", "88");
  await expect(page.getByText("HIGH", { exact: true })).toBeVisible();
  await page.screenshot({ path: "docs/screenshots/dashboard.png" });
  expect(errors).toEqual([]);
});

test("every page loads without errors", async ({ page }) => {
  const errors = watchErrors(page);
  await installBridge(page);
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Main" });
  for (const name of ["Processes", "Storage", "Diagnosis", "Actions", "History", "Settings", "Dashboard"]) {
    await nav.getByRole("link", { name }).click();
    await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
  }
  expect(errors).toEqual([]);
});

test("no page needs horizontal scrolling at the minimum window size", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 600 });
  await installBridge(page);
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Main" });
  for (const name of ["Dashboard", "Processes", "Storage", "Diagnosis", "Actions", "History", "Settings"]) {
    await nav.getByRole("link", { name }).click();
    await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `${name} overflows horizontally`).toBeLessThanOrEqual(0);
  }
});

test("diagnosis: a medium-risk action needs confirmation and is previewed first", async ({ page }) => {
  const errors = watchErrors(page);
  await installBridge(
    page,
    {
      preview_action: {
        actionId: "wsl.shutdown",
        title: "Shut down WSL",
        summary: "Stops every WSL distribution and the Docker Desktop engine.",
        risk: "MEDIUM",
        items: [{ label: "wsl --shutdown", detail: null, bytes: null }],
        estimatedBenefitBytes: null,
        warnings: ["Running terminals and dev servers inside WSL will stop."],
        requiresConfirmation: true,
        requiresDoubleConfirmation: false,
        reversible: false,
        reversibility: "IRREVERSIBLE",
      },
      execute_action: { id: 1, actionId: "wsl.shutdown", name: "Shut down WSL", atMs: 1, risk: "MEDIUM", automated: false, success: true, summary: "WSL was shut down.", freedBytes: null, rolledBack: false, reversible: false, reversibility: "IRREVERSIBLE", status: "SUCCESS", verification: { state: "VERIFIED", detail: "Confirmed." }, measurement: null },
    },
  );
  await page.goto("/#/diagnosis");
  await page.getByRole("button", { name: "Shut down WSL" }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Stops every WSL distribution")).toBeVisible();
  await expect(dialog.getByText("Running terminals and dev servers inside WSL will stop.")).toBeVisible();
  expect(await calls(page, "execute_action")).toHaveLength(0);

  const run = dialog.getByRole("button", { name: "Run" });
  await expect(run).toBeDisabled();
  await dialog.getByLabel(/I have read the preview/).check();
  await expect(run).toBeEnabled();
  await run.click();
  await expect(dialog.getByText("WSL was shut down.")).toBeVisible();
  const exec = await calls(page, "execute_action");
  expect(exec[0].args).toMatchObject({ actionId: "wsl.shutdown", confirmed: true, doubleConfirmed: false });
  expect(errors).toEqual([]);
});

test("escape closes the dialog without running anything", async ({ page }) => {
  await installBridge(page, {
    preview_action: { actionId: "wsl.shutdown", title: "Shut down WSL", summary: "x", risk: "MEDIUM", items: [{ label: "wsl --shutdown", detail: null, bytes: null }], estimatedBenefitBytes: null, warnings: [], requiresConfirmation: true, requiresDoubleConfirmation: false, reversible: false, reversibility: "IRREVERSIBLE" },
  });
  await page.goto("/#/diagnosis");
  await page.getByRole("button", { name: "Shut down WSL" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
  expect(await calls(page, "execute_action")).toHaveLength(0);
});

test("storage: scan, review node_modules, double confirmation, row disappears", async ({ page }) => {
  const errors = watchErrors(page);
  const entry = { path: "S:\\proj\\node_modules", name: "node_modules", isDir: true, sizeBytes: 6.4 * GB, fileCount: 52000, lastModifiedSecs: 1_700_000_000, kind: "nodeModules", kindLabel: "Developer dependency", safety: "REQUIRES_REVIEW", reason: "Reinstallable." };
  const temp = { ...entry, path: "C:\\Users\\a\\AppData\\Local\\Temp", name: "Temp", kind: "tempCache", kindLabel: "Temporary files", safety: "SAFE_TO_CLEAN", sizeBytes: 3 * GB, fileCount: 900 };
  const done = { running: false, progress: { phase: "Done", currentPath: "", entriesFound: 2, itemsVisited: 5000 }, error: null, result: { startedAtMs: 1, durationMs: 4200, complete: true, unreadable: 0, notes: [], entries: [entry, temp] } };
  await installBridge(
    page,
    {
      start_storage_scan: true,
      preview_action: { actionId: "cleanup.path", title: "Delete node_modules", summary: "6.4 GB", risk: "DESTRUCTIVE", items: [{ label: "S:\\proj\\node_modules", detail: "NodeModules", bytes: 6.4 * GB }], estimatedBenefitBytes: 6.4 * GB, warnings: ["Deleted permanently. It is not moved to the Recycle Bin."], requiresConfirmation: true, requiresDoubleConfirmation: true, reversible: false, reversibility: "IRREVERSIBLE" },
      execute_action: { id: 1, actionId: "cleanup.path", name: "Delete a scanned item", atMs: 1, risk: "DESTRUCTIVE", automated: false, success: true, summary: "Deleted and freed 6.4 GB.", freedBytes: 6.4 * GB, rolledBack: false, reversible: false, reversibility: "IRREVERSIBLE", status: "SUCCESS", verification: { state: "VERIFIED", detail: "Confirmed." }, measurement: null },
    },
    {
      // Idle until the user starts a scan, then complete.
      get_storage_scan: `(() => { let started = false; return () => { if (window.__started) return ${JSON.stringify(done)}; return { running: false, progress: { phase: "", currentPath: "", entriesFound: 0, itemsVisited: 0 }, result: null, error: null }; }; })()`,
      start_storage_scan: `() => { window.__started = true; return true; }`,
    },
  );
  await page.goto("/#/storage");
  await expect(page.getByText(/No scan yet/)).toBeVisible();
  await page.getByRole("button", { name: "Scan storage" }).click();

  await expect(page.getByText("Safe to clean").first()).toBeVisible();
  await page.screenshot({ path: "docs/screenshots/storage-overview.png" });
  await page.getByRole("tab", { name: "Cleanup candidates" }).click();
  await page.screenshot({ path: "docs/screenshots/storage-cleanup.png" });
  await page.getByRole("button", { name: /Review and delete node_modules/ }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Delete node_modules")).toBeVisible();
  const run = dialog.getByRole("button", { name: "Delete permanently" });
  await expect(run).toBeDisabled();
  await dialog.getByLabel(/I have read the preview/).check();
  await expect(run).toBeDisabled();
  await dialog.getByLabel(/I understand it is permanent/).check();
  await run.click();
  await expect(dialog.getByText("Deleted and freed 6.4 GB.")).toBeVisible();
  const exec = await calls(page, "execute_action");
  expect(exec[0].args).toMatchObject({ actionId: "cleanup.path", params: { path: "S:\\proj\\node_modules" }, confirmed: true, doubleConfirmed: true });
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("button", { name: /Review and delete node_modules/ })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("history draws charts for the selected range", async ({ page }) => {
  const errors = watchErrors(page);
  await installBridge(page);
  await page.goto("/#/history");
  await expect(page.getByText("RAM usage (%)")).toBeVisible();
  await expect(page.locator(".recharts-surface").first()).toBeVisible();
  expect(await page.locator(".recharts-surface").count()).toBeGreaterThanOrEqual(3);
  await page.getByRole("button", { name: "24 h" }).click();
  await expect.poll(async () => (await calls(page, "get_history")).at(-1)?.args.rangeSecs).toBe(24 * 3600);
  await page.screenshot({ path: "docs/screenshots/history.png" });
  expect(errors).toEqual([]);
});

test("history says so when the database is unavailable", async ({ page }) => {
  await installBridge(page, { get_history: { available: false, items: [] } });
  await page.goto("/#/history");
  await expect(page.getByText(/History is unavailable/)).toBeVisible();
});

test("dark theme applies, persists across reloads and keeps text readable", async ({ page }) => {
  await installBridge(page);
  await page.goto("/#/settings");
  await page.getByRole("button", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.reload();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.goto("/#/");
  await expect(page.getByText("Problem detected")).toBeVisible();
  await page.screenshot({ path: "docs/screenshots/dashboard-dark.png" });
});

test("settings: AI is off by default and the key field is a password input", async ({ page }) => {
  await installBridge(page);
  await page.goto("/#/settings");
  await expect(page.getByRole("switch", { name: "Enable AI assistance" })).toHaveAttribute("aria-checked", "false");
  await expect(page.getByPlaceholder("Paste a key to store it")).toHaveAttribute("type", "password");
  await expect(page.getByRole("switch", { name: "Automatically perform safe optimizations" })).toHaveAttribute("aria-checked", "false");
  expect(await calls(page, "ask_ai")).toHaveLength(0);
});

test("the tray can open a page: a pending route is claimed on startup", async ({ page }) => {
  await installBridge(page, { take_pending_route: "/diagnosis" });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Diagnosis", level: 1 })).toBeVisible();
});

test("a `navigate` event from Rust changes the page", async ({ page }) => {
  await installBridge(page);
  await page.goto("/");
  await expect(page.getByText("Problem detected")).toBeVisible();
  await page.evaluate(() => (window as any).__emit("navigate", "/settings"));
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
});

test("the app still renders when every backend call fails", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    (window as any).__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => { throw "backend unavailable"; } };
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Dashboard", level: 1 })).toBeVisible();
  await expect(page.getByText(/backend unavailable/).first()).toBeVisible();
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Storage" }).click();
  await expect(page.getByRole("heading", { name: "Storage", level: 1 })).toBeVisible();
});
