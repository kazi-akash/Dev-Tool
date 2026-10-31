import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { DiagnosisPage } from "@/features/diagnosis/DiagnosisPage";
import { SettingsProvider } from "@/lib/settings";
import { preview, report } from "./fixtures";
import { ipc } from "./ipc-mock";

function mount(aiEnabled = false) {
  if (aiEnabled) {
    localStorage.setItem("devpilot.settings.v1", JSON.stringify({ ai: { enabled: true, kind: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-sonnet-5-5" } }));
  }
  ipc.on("run_diagnosis", report());
  return render(
    <SettingsProvider>
      <MemoryRouter>
        <DiagnosisPage />
      </MemoryRouter>
    </SettingsProvider>,
  );
}

describe("diagnosis page", () => {
  it("explains why, with contributors, confidence, evidence and recommendations", async () => {
    mount();
    expect(await screen.findByText("Memory pressure detected")).toBeInTheDocument();
    expect(screen.getByText(/account for approximately 8.9 GB/)).toBeInTheDocument();
    expect(screen.getByText("Confidence 91%")).toBeInTheDocument();
    expect(screen.getByText("30.0% of RAM", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Primary cause:", { exact: false }).parentElement).toHaveTextContent("Docker/WSL2");
    // Each recommendation: problem, explanation, benefit, risk, and an action or manual steps.
    expect(screen.getByText("Container redis-idle looks idle")).toBeInTheDocument();
    expect(screen.getByText(/It uses 900 MB/)).toBeInTheDocument();
    expect(screen.getByText("About 900.0 MB of RAM")).toBeInTheDocument();
    expect(screen.getAllByText("low risk").length).toBe(2);
    expect(screen.getByRole("button", { name: "Stop container redis-idle" })).toBeInTheDocument();
    expect(screen.getByText(/memory=6GB/)).toBeInTheDocument();
  });

  it("runs a recommendation's action only through the preview dialog", async () => {
    const user = userEvent.setup();
    ipc.on("preview_action", preview({ actionId: "docker.stop_container", title: "Stop container redis-idle", risk: "LOW", requiresConfirmation: false, requiresDoubleConfirmation: false, warnings: [], estimatedBenefitBytes: null, reversible: true }));
    ipc.on("execute_action", { id: 1, actionId: "docker.stop_container", name: "Stop Docker container", atMs: 1, risk: "LOW", automated: false, success: true, summary: "Stopped container redis-idle.", freedBytes: null, rolledBack: false, reversible: true });
    mount();
    await user.click(await screen.findByRole("button", { name: "Stop container redis-idle" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Stop container redis-idle")).toBeInTheDocument();
    expect(ipc.calledWith("execute_action")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Run" }));
    expect(await within(dialog).findByText("Stopped container redis-idle.")).toBeInTheDocument();
    expect(ipc.calledWith("execute_action")[0].args).toMatchObject({ actionId: "docker.stop_container", params: { container: "redis-idle" }, confirmed: false, doubleConfirmed: false });
    // The report is refreshed after an action ran.
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(ipc.calledWith("run_diagnosis").length).toBeGreaterThanOrEqual(2);
  });

  it("medium-risk actions stay disabled until confirmed", async () => {
    const user = userEvent.setup();
    ipc.on("preview_action", preview({ actionId: "wsl.shutdown", title: "Shut down WSL", risk: "MEDIUM", requiresConfirmation: true, requiresDoubleConfirmation: false, reversible: false }));
    ipc.on("run_diagnosis", report({}, { ...report().headline, recommendations: [{ ...report().headline.recommendations[0], action: { actionId: "wsl.shutdown", params: {}, label: "Shut down WSL" } }] }));
    render(
      <SettingsProvider>
        <MemoryRouter>
          <DiagnosisPage />
        </MemoryRouter>
      </SettingsProvider>,
    );
    await user.click(await screen.findByRole("button", { name: "Shut down WSL" }));
    const run = await screen.findByRole("button", { name: "Run" });
    expect(run).toBeDisabled();
    await user.click(screen.getByLabelText(/I have read the preview/));
    expect(run).toBeEnabled();
  });

  it("shows an error when the backend cannot diagnose", async () => {
    ipc.on("run_diagnosis", () => {
      throw "Monitoring has not produced a sample yet.";
    });
    render(
      <SettingsProvider>
        <MemoryRouter>
          <DiagnosisPage />
        </MemoryRouter>
      </SettingsProvider>,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("not produced a sample");
  });
});

describe("ai assistant", () => {
  it("is off by default and says nothing is sent", async () => {
    mount();
    expect(await screen.findByText(/Off\. The diagnosis above is produced locally/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Ask AI" })).toBeNull();
    expect(ipc.calledWith("ask_ai")).toHaveLength(0);
  });

  it("sends the opt-in flag and config, and shows the answer as plain text", async () => {
    const user = userEvent.setup();
    ipc.on("ask_ai", { text: "Close Docker. <b>bold</b> <img src=x onerror=alert(1)>" });
    mount(true);
    await user.type(await screen.findByLabelText("Question for the AI"), "why slow?");
    await user.click(screen.getByRole("button", { name: "Ask AI" }));
    const answer = await screen.findByLabelText("AI answer");
    expect(answer).toHaveTextContent("Close Docker. <b>bold</b> <img src=x onerror=alert(1)>");
    expect(answer.querySelector("b, img")).toBeNull();
    expect(ipc.calledWith("ask_ai")[0].args).toMatchObject({ enabled: true, config: { kind: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-sonnet-5-5" }, question: "why slow?" });
  });

  it("lets the user inspect exactly what would be sent", async () => {
    const user = userEvent.setup();
    ipc.on("preview_ai_payload", { system: { ram: "1/2 GB", cpu: "1%", pressure: "NORMAL", trend: "STABLE", disks: [] }, workloads: [], topProcesses: [], diagnosisStatus: "healthy", findings: [] });
    mount(true);
    await user.click(await screen.findByRole("button", { name: "Show what will be sent" }));
    expect(await screen.findByLabelText("AI payload preview")).toHaveTextContent('"diagnosisStatus": "healthy"');
    expect(ipc.calledWith("ask_ai")).toHaveLength(0);
  });

  it("shows provider errors without breaking the page", async () => {
    const user = userEvent.setup();
    ipc.on("ask_ai", () => {
      throw { code: "auth", message: "The provider rejected the API key." };
    });
    mount(true);
    await user.click(await screen.findByRole("button", { name: "Ask AI" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("rejected the API key");
    expect(screen.getByText("Memory pressure detected")).toBeInTheDocument();
  });
});
