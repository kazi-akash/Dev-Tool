import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StoragePage } from "@/features/storage/StoragePage";
import { overview, sortEntries, viewEntries } from "@/features/storage/views";
import { GB, entry, preview, scanStatus } from "./fixtures";
import { ipc } from "./ipc-mock";

const nm = entry();
const temp = entry({ path: "C:\\Users\\a\\AppData\\Local\\Temp", name: "Temp", kind: "tempCache", kindLabel: "Temporary files", safety: "SAFE_TO_CLEAN", sizeBytes: 3 * GB, fileCount: 900, lastModifiedSecs: 1_700_500_000 });
const next = entry({ path: "S:\\proj\\.next", name: ".next", kind: "buildOutput", kindLabel: "Build output", safety: "SAFE_TO_REVIEW", sizeBytes: 1 * GB, fileCount: 12000, lastModifiedSecs: 1_699_000_000 });
const sdk = entry({ path: "C:\\Users\\a\\AppData\\Local\\Android\\Sdk", name: "Sdk", kind: "androidSdk", kindLabel: "Android SDK", safety: "DO_NOT_TOUCH", sizeBytes: 20 * GB, fileCount: 80000 });
const iso = entry({ path: "C:\\Users\\a\\Downloads\\os.iso", name: "os.iso", isDir: false, kind: "diskImage", kindLabel: "Disk image", safety: "REQUIRES_REVIEW", sizeBytes: 4 * GB, fileCount: 1 });
const all = [sdk, nm, iso, temp, next];

describe("storage views (pure)", () => {
  it("sorts by size, modified date and file count", () => {
    expect(sortEntries(all, "size").map((e) => e.name)[0]).toBe("Sdk");
    expect(sortEntries(all, "modified")[0].name).toBe("Temp");
    expect(sortEntries(all, "files")[0].name).toBe("Sdk");
    expect(sortEntries(all, "files").at(-1)!.name).toBe("os.iso");
  });

  it("splits folders, files, developer caches and cleanup candidates", () => {
    expect(viewEntries(all, "folders", "size").every((e) => e.isDir)).toBe(true);
    expect(viewEntries(all, "files", "size").map((e) => e.name)).toEqual(["os.iso"]);
    expect(viewEntries(all, "caches", "size").map((e) => e.name)).toEqual(["Sdk", "node_modules", "Temp", ".next"]);
    const cleanup = viewEntries(all, "cleanup", "size").map((e) => e.name);
    expect(cleanup).not.toContain("Sdk");
    expect(cleanup).toContain("Temp");
  });

  it("totals what could be recovered by safety class", () => {
    const o = overview(all);
    expect(o.cleanBytes).toBe(3 * GB);
    expect(o.recoverableBytes).toBe(4 * GB);
    expect(o.reviewBytes).toBe(nm.sizeBytes + 4 * GB);
    expect(o.protectedBytes).toBe(20 * GB);
    expect(o.byKind[0].kind).toBe("androidSdk");
    expect(overview([])).toMatchObject({ totalBytes: 0, byKind: [] });
  });
});

describe("storage page", () => {
  it("offers a scan, then shows progress and results", async () => {
    const user = userEvent.setup();
    let state = scanStatus([], { result: null });
    ipc.on("get_storage_scan", () => state);
    ipc.on("start_storage_scan", () => {
      state = scanStatus(all);
      return true;
    });
    render(<StoragePage />);
    expect(await screen.findByText(/No scan yet/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Scan storage" }));
    expect(ipc.calledWith("start_storage_scan")).toHaveLength(1);
    expect(await screen.findByText("Safe to clean")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Scan again" })).toBeInTheDocument();
  });

  it("shows live progress while scanning and allows cancel", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus([], { running: true, result: null, progress: { phase: "Project artifacts", currentPath: "S:\\Client Projects", entriesFound: 3, itemsVisited: 52000 } }));
    ipc.on("cancel_storage_scan", undefined);
    render(<StoragePage />);
    expect(await screen.findByText("Project artifacts")).toBeInTheDocument();
    expect(screen.getByText(/52,000 items checked · 3 found/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Cancel scan" }));
    expect(ipc.calledWith("cancel_storage_scan")).toHaveLength(1);
  });

  it("lists cleanup candidates with safety labels and never offers do-not-touch items", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Developer caches" }));
    const rows = screen.getAllByRole("row").slice(1);
    const sdkRow = rows.find((r) => within(r).queryByText("Sdk"))!;
    expect(within(sdkRow).getByText("Do not touch")).toBeInTheDocument();
    expect(within(sdkRow).queryByRole("button")).toBeNull();
    const nmRow = rows.find((r) => within(r).queryByText("node_modules"))!;
    expect(within(nmRow).getByText("Requires review")).toBeInTheDocument();
    expect(within(nmRow).getByRole("button", { name: /Review and delete node_modules/ })).toBeInTheDocument();
  });

  it("changes sort order", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Largest folders" }));
    const names = () => screen.getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("cell")[0].textContent ?? "");
    expect(names()[0]).toContain("Sdk");
    await user.click(screen.getByRole("button", { name: "File count" }));
    expect(names()[0]).toContain("Sdk");
    await user.click(screen.getByRole("button", { name: "Modified" }));
    expect(names()[0]).toContain("Temp");
  });

  it("destructive cleanup needs two confirmations, previews first, and removes the row afterwards", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    ipc.on("preview_action", preview());
    ipc.on("execute_action", () => ({ id: 1, actionId: "cleanup.path", name: "Delete a scanned item", atMs: 1, risk: "DESTRUCTIVE", automated: false, success: true, summary: "Deleted and freed 6.4 GB.", freedBytes: 6.4 * GB, rolledBack: false, reversible: false, reversibility: "IRREVERSIBLE", status: "SUCCESS", verification: { state: "VERIFIED", detail: "Confirmed." }, measurement: null }));
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Cleanup candidates" }));
    await user.click(screen.getByRole("button", { name: /Review and delete node_modules/ }));

    // The preview is shown before anything can run.
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(await screen.findByText("Delete node_modules")).toBeInTheDocument();
    expect(screen.getByText(/not moved to the Recycle Bin/)).toBeInTheDocument();
    expect(ipc.calledWith("execute_action")).toHaveLength(0);

    const run = screen.getByRole("button", { name: "Delete permanently" });
    expect(run).toBeDisabled();
    await user.click(screen.getByLabelText(/I have read the preview/));
    expect(run).toBeDisabled();
    await user.click(screen.getByLabelText(/I understand it is permanent/));
    expect(run).toBeEnabled();
    await user.click(run);

    expect(await screen.findByText("Deleted and freed 6.4 GB.")).toBeInTheDocument();
    const exec = ipc.calledWith("execute_action")[0].args!;
    expect(exec).toMatchObject({ actionId: "cleanup.path", params: { path: nm.path }, confirmed: true, doubleConfirmed: true });

    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Review and delete node_modules/ })).toBeNull());
  });

  it("low-risk cleanup needs no confirmation and Cancel runs nothing", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    ipc.on("preview_action", preview({ title: "Delete Temp", risk: "LOW", requiresConfirmation: false, requiresDoubleConfirmation: false, warnings: [] }));
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Cleanup candidates" }));
    await user.click(screen.getByRole("button", { name: /Review and delete Temp/ }));
    expect(await screen.findByText("Delete Temp")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(ipc.calledWith("execute_action")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the engine's refusal instead of running", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    ipc.on("preview_action", () => {
      throw { code: "refused", message: "This location is protected (system or personal folder)." };
    });
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Cleanup candidates" }));
    await user.click(screen.getAllByRole("button", { name: /Review and delete/ })[0]);
    expect(await screen.findByRole("alert")).toHaveTextContent("protected");
    expect(screen.queryByRole("button", { name: "Run" })).toBeNull();
  });

  it("explains partial results", async () => {
    ipc.on("get_storage_scan", scanStatus(all, { result: { ...scanStatus(all).result!, complete: false, notes: ["Scan was cancelled; results are partial."] } }));
    render(<StoragePage />);
    expect(await screen.findByText(/results are partial/)).toBeInTheDocument();
  });
});

describe("deep AppData scan", () => {
  const slack = entry({ path: "C:/Users/a/AppData/Local/Slack", name: "Slack", kind: "appData", kindLabel: "Application data", safety: "DO_NOT_TOUCH", sizeBytes: 2.1 * GB, fileCount: 4000 });

  it("is never started by a normal scan and asks before it starts", async () => {
    const user = userEvent.setup();
    ipc.on("get_storage_scan", scanStatus(all));
    ipc.on("start_storage_scan", true);
    ipc.on("start_deep_appdata_scan", true);
    render(<StoragePage />);
    await screen.findByText("Safe to clean");
    await user.click(screen.getByRole("button", { name: "Scan again" }));
    expect(ipc.calledWith("start_deep_appdata_scan")).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: "Deep scan AppData" }));
    expect(screen.getByText("This can take several minutes.")).toBeInTheDocument();
    expect(screen.getByText(/pauses while memory is critical/)).toBeInTheDocument();
    expect(ipc.calledWith("start_deep_appdata_scan")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(ipc.calledWith("start_deep_appdata_scan")).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: "Deep scan AppData" }));
    await user.click(screen.getByRole("button", { name: "Start deep scan" }));
    expect(ipc.calledWith("start_deep_appdata_scan")).toHaveLength(1);
  });

  it("lists app folders without any way to delete them", async () => {
    ipc.on("get_storage_scan", scanStatus(all, { deepResult: { startedAtMs: 1, durationMs: 1000, complete: true, unreadable: 0, notes: ["Deep scan: only per-app folders over 100 MB are listed."], entries: [slack] } }));
    render(<StoragePage />);
    expect(await screen.findByText("Slack")).toBeInTheDocument();
    expect(screen.getByText("2.1 GB")).toBeInTheDocument();
    expect(screen.getByText(/DevPilot does not offer to delete application data/)).toBeInTheDocument();
    const card = screen.getByText("Deep scan of AppData (optional)").closest("div")!.parentElement!;
    expect(within(card).queryByRole("button", { name: /delete|clean|remove/i })).not.toBeInTheDocument();
  });

  it("shows when the scan is paused and offers to cancel it", async () => {
    ipc.on("get_storage_scan", scanStatus(all, { running: true, deep: true, progress: { phase: "Paused: memory pressure is critical", currentPath: "C:/x", entriesFound: 0, itemsVisited: 5000 } }));
    ipc.on("cancel_storage_scan", undefined);
    render(<StoragePage />);
    expect((await screen.findAllByText("Paused: memory pressure is critical")).length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: "Deep scan AppData" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Cancel/ }).length).toBeGreaterThan(0);
  });

  it("a standard scan in progress disables starting a deep one", async () => {
    ipc.on("get_storage_scan", scanStatus(all, { running: true, deep: false }));
    render(<StoragePage />);
    expect(await screen.findByRole("button", { name: "Deep scan AppData" })).toBeDisabled();
  });
});

describe("storage table virtualization", () => {
  it("keeps the DOM small however many rows were revealed", async () => {
    const user = userEvent.setup();
    const many = Array.from({ length: 600 }, (_, i) => entry({ path: `S:/p${i}/node_modules`, name: `node_modules_${i}`, sizeBytes: (700 - i) * 1024 * 1024, safety: "SAFE_TO_REVIEW", kind: "buildOutput", kindLabel: "Build output" }));
    ipc.on("get_storage_scan", scanStatus(many));
    render(<StoragePage />);
    await user.click(await screen.findByRole("tab", { name: "Largest folders" }));
    // 100 rows: all rendered.
    expect(screen.getAllByRole("row")).toHaveLength(101);
    for (let i = 0; i < 4; i++) await user.click(screen.getByRole("button", { name: /Show \d+ more/ }));
    // 500 revealed, but only a window is in the DOM.
    const rows = screen.getAllByRole("row").length;
    expect(rows).toBeLessThan(60);
    expect(screen.getByText("node_modules_0")).toBeInTheDocument();
    expect(screen.queryByText("node_modules_300")).not.toBeInTheDocument();
  });
});
