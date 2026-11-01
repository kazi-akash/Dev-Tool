import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ActionDialog } from "@/components/ActionDialog";
import { ActionResult } from "@/components/ActionResult";
import type { ActionRecord } from "@/types/api";
import { GB, preview } from "./fixtures";
import { ipc } from "./ipc-mock";

const record = (over: Partial<ActionRecord> = {}): ActionRecord => ({
  id: 1,
  actionId: "docker.stop_container",
  name: "Stop Docker container",
  atMs: 1,
  risk: "LOW",
  automated: false,
  success: true,
  summary: "Stopped container nidustime-api.",
  freedBytes: null,
  rolledBack: false,
  reversible: true,
  reversibility: "REVERSIBLE",
  status: "SUCCESS",
  verification: { state: "VERIFIED", detail: "nidustime-api is no longer in the running container list." },
  measurement: {
    before: { ramUsedBytes: 15.1 * GB, ramAvailableBytes: 1 * GB, diskFreeBytes: null },
    after: { ramUsedBytes: 13.4 * GB, ramAvailableBytes: 2.7 * GB, diskFreeBytes: null },
    ramRecoveredBytes: 1.7 * GB,
    diskRecoveredBytes: null,
    note: "Measured system-wide before and after; other programs also change memory and disk use.",
  },
  ...over,
});

const li = (text: string) => screen.getByText((_, el) => el?.tagName === "LI" && (el.textContent ?? "").includes(text));

describe("action result", () => {
  it("shows status, class, what was verified and the measured before/after", () => {
    render(<ActionResult record={record()} />);
    expect(screen.getByText("Success")).toBeInTheDocument();
    expect(screen.getByText("Reversible")).toBeInTheDocument();
    expect(screen.getByText(/no longer in the running container list/)).toBeInTheDocument();
    expect(li("Available RAM: 1.0 GB → 2.7 GB (+1.7 GB)")).toBeInTheDocument();
    expect(screen.getByText(/\+1.7 GB/)).toBeInTheDocument();
    expect(screen.getByText(/other programs also change memory/)).toBeInTheDocument();
  });

  it("reports a run whose effect was not seen as such, never as success", () => {
    render(<ActionResult record={record({ status: "NOT_EFFECTIVE", verification: { state: "NOT_CONFIRMED", detail: "PID 7 is still running." } })} />);
    expect(screen.getByText("No visible effect")).toBeInTheDocument();
    expect(screen.queryByText("Success")).not.toBeInTheDocument();
    expect(screen.getByText(/PID 7 is still running/)).toBeInTheDocument();
  });

  it("shows memory that grew as negative and unreadable values as not measurable", () => {
    const m = record().measurement!;
    render(<ActionResult record={record({ measurement: { ...m, after: { ...m.after, ramAvailableBytes: 1.5 * GB }, ramRecoveredBytes: -1.2 * GB } })} />);
    expect(li("(-1.2 GB)")).toBeInTheDocument();
  });

  it("says not measurable when the OS gave no reading", () => {
    const m = record().measurement!;
    render(<ActionResult record={record({ measurement: { ...m, before: { ramUsedBytes: null, ramAvailableBytes: null, diskFreeBytes: null }, ramRecoveredBytes: null } })} />);
    expect(li("Available RAM: not measurable")).toBeInTheDocument();
  });

  it("irreversible runs are labelled permanent", () => {
    render(<ActionResult record={record({ reversibility: "IRREVERSIBLE", reversible: false })} />);
    expect(screen.getByText("Irreversible")).toBeInTheDocument();
  });

  it("an unverifiable action says so", () => {
    render(<ActionResult record={record({ status: "UNVERIFIED", verification: { state: "UNVERIFIABLE", detail: "No automatic check exists for this action." } })} />);
    expect(screen.getByText("Done, not verified")).toBeInTheDocument();
  });
});

describe("action dialog lifecycle", () => {
  const recycle = () =>
    preview({
      title: "Move to Recycle Bin: node_modules",
      reversible: true,
      reversibility: "RECOVERABLE",
      estimatedBenefitBytes: null,
      warnings: ["Moved to the Recycle Bin, not deleted. Disk space is released only when the Recycle Bin is emptied."],
    });

  it("previews the recycle-bin move, offers permanent deletion explicitly, and re-previews with it", async () => {
    const user = userEvent.setup();
    ipc.on("preview_action", (args) => ((args?.params as Record<string, unknown>).permanent ? preview() : recycle()));
    render(<ActionDialog actionId="cleanup.path" params={{ path: "S:\\proj\\node_modules" }} onClose={() => {}} />);
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("Recoverable")).toBeInTheDocument();
    expect(within(dialog).getByText(/Disk space is released only when the Recycle Bin is emptied/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Delete permanently" })).not.toBeInTheDocument();

    await user.click(within(dialog).getByLabelText(/Delete permanently instead of moving/));
    expect(await within(dialog).findByText("Irreversible")).toBeInTheDocument();
    expect(await within(dialog).findByRole("button", { name: "Delete permanently" })).toBeInTheDocument();
    const last = ipc.calledWith("preview_action").at(-1)!.args as { params: Record<string, unknown> };
    expect(last.params).toMatchObject({ path: "S:\\proj\\node_modules", permanent: true });
  });

  it("nothing runs from the dialog until confirmed, then the verified result is shown", async () => {
    const user = userEvent.setup();
    ipc.on("preview_action", recycle());
    ipc.on("execute_action", record({ actionId: "cleanup.path", summary: "Moved node_modules (6.4 GB) to the Recycle Bin.", reversibility: "RECOVERABLE", verification: { state: "VERIFIED", detail: "The item is gone from its location and was found in the Recycle Bin." } }));
    render(<ActionDialog actionId="cleanup.path" params={{ path: "S:\\proj\\node_modules" }} onClose={() => {}} />);
    const dialog = await screen.findByRole("dialog");
    expect(ipc.calledWith("execute_action")).toHaveLength(0);
    await user.click(await within(dialog).findByLabelText(/I have read the preview/));
    await user.click(within(dialog).getByLabelText(/I understand it is permanent/));
    await user.click(within(dialog).getByRole("button", { name: "Run" }));
    expect(await within(dialog).findByText(/found in the Recycle Bin/)).toBeInTheDocument();
    expect(within(dialog).getAllByText("Recoverable")).toHaveLength(2); // preview badge + result badge
  });
});
