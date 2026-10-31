import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { SettingsProvider } from "@/lib/settings";
import { ipc } from "./ipc-mock";

function mount() {
  ipc.on("ai_key_status", false);
  for (const c of ["set_monitor_interval", "set_pressure_thresholds", "set_disk_thresholds", "set_safe_automation"]) ipc.on(c, undefined);
  return render(
    <SettingsProvider>
      <SettingsPage />
    </SettingsProvider>,
  );
}

describe("settings: backend-owned values", () => {
  it("pushes stored values to Rust on startup", async () => {
    localStorage.setItem("devpilot.settings.v1", JSON.stringify({ monitorIntervalSec: 5, safeAutomation: true }));
    mount();
    await waitFor(() => expect(ipc.calledWith("set_monitor_interval")[0].args).toEqual({ secs: 5 }));
    expect(ipc.calledWith("set_safe_automation")[0].args).toEqual({ enabled: true });
    expect(ipc.calledWith("set_pressure_thresholds")[0].args).toEqual({ moderate: 70, high: 85, critical: 92 });
  });

  it("toggles safe automation and tells Rust", async () => {
    const user = userEvent.setup();
    mount();
    const sw = screen.getByRole("switch", { name: "Automatically perform safe optimizations" });
    expect(sw).toHaveAttribute("aria-checked", "false");
    await user.click(sw);
    await waitFor(() => expect(ipc.calledWith("set_safe_automation").at(-1)!.args).toEqual({ enabled: true }));
    expect(screen.getByText(/never deletes other files, closes programs, edits the registry/i)).toBeInTheDocument();
  });
});

describe("settings: thresholds", () => {
  it("applies valid thresholds through Rust", async () => {
    const user = userEvent.setup();
    mount();
    ipc.on("set_pressure_thresholds", (a) => ({ moderate: a!.moderate, high: a!.high, critical: a!.critical }));
    ipc.on("set_disk_thresholds", (a) => ({ warningFree: a!.warningFree, highFree: a!.highFree, criticalFree: a!.criticalFree }));
    const high = screen.getByLabelText("High");
    await user.clear(high);
    await user.type(high, "80");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await screen.findByText("Thresholds applied.")).toBeInTheDocument();
    expect(ipc.calledWith("set_pressure_thresholds").at(-1)!.args).toEqual({ moderate: 70, high: 80, critical: 92 });
    expect(JSON.parse(localStorage.getItem("devpilot.settings.v1")!).pressureThresholds.high).toBe(80);
  });

  it("shows Rust's validation message and does not store invalid values", async () => {
    const user = userEvent.setup();
    mount();
    ipc.on("set_pressure_thresholds", () => {
      throw "thresholds must satisfy 0 < moderate < high < critical <= 100 (1 point apart)";
    });
    const crit = screen.getByLabelText("Critical");
    await user.clear(crit);
    await user.type(crit, "60");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("0 < moderate < high < critical");
    expect(JSON.parse(localStorage.getItem("devpilot.settings.v1")!).pressureThresholds.critical).toBe(92);
  });

  it("rejects non-numeric input before calling Rust", async () => {
    const user = userEvent.setup();
    mount();
    const before = ipc.calledWith("set_pressure_thresholds").length;
    const mod = screen.getByLabelText("Moderate");
    await user.clear(mod);
    await user.type(mod, "abc");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("must be a number");
    expect(ipc.calledWith("set_pressure_thresholds").length).toBe(before);
  });
});

describe("settings: AI", () => {
  it("is off by default and a key is never displayed", async () => {
    mount();
    expect(screen.getByRole("switch", { name: "Enable AI assistance" })).toHaveAttribute("aria-checked", "false");
    expect(await screen.findByText(/\(none stored\)/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Paste a key to store it")).toHaveAttribute("type", "password");
  });

  it("stores a key in the backend and clears the field", async () => {
    const user = userEvent.setup();
    mount();
    ipc.on("set_ai_key", undefined);
    const input = screen.getByPlaceholderText("Paste a key to store it");
    await user.type(input, "sk-test-abcdef");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText(/saved in the Windows Credential Manager/)).toBeInTheDocument();
    expect(ipc.calledWith("set_ai_key")[0].args).toEqual({ key: "sk-test-abcdef" });
    expect(input).toHaveValue("");
    // The key is not kept in localStorage.
    expect(localStorage.getItem("devpilot.settings.v1")).not.toContain("sk-test-abcdef");
  });

  it("presets fill provider, URL and model; Ollama needs no key", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Ollama (local)" }));
    expect(screen.getByDisplayValue("http://localhost:11434/v1")).toBeInTheDocument();
    expect(screen.getByDisplayValue("llama3.1")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Anthropic" }));
    expect(screen.getByDisplayValue("https://api.anthropic.com")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("devpilot.settings.v1")!).ai.kind).toBe("anthropic");
  });

  it("shows the exact payload on request", async () => {
    const user = userEvent.setup();
    mount();
    ipc.on("preview_ai_payload", { system: { ram: "x", cpu: "1%", pressure: "NORMAL", trend: "STABLE", disks: [] }, workloads: [], topProcesses: [], diagnosisStatus: "healthy", findings: [] });
    await user.click(screen.getByRole("button", { name: "Show exactly what would be sent" }));
    expect(await screen.findByLabelText("AI payload preview")).toHaveTextContent("diagnosisStatus");
  });
});
