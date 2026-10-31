import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { App } from "@/app/App";
import { ipc } from "./ipc-mock";

describe("tray navigation", () => {
  it("opens the route the tray requested while the window was closed", async () => {
    ipc.on("take_pending_route", "/storage");
    ipc.on("get_storage_scan", { running: false, progress: { phase: "", currentPath: "", entriesFound: 0, itemsVisited: 0 }, result: null, error: null });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Storage" })).toBeInTheDocument();
  });

  it("follows `navigate` events while the window is open", async () => {
    ipc.on("take_pending_route", null);
    render(<App />);
    await waitFor(() => expect(ipc.calledWith("take_pending_route")).toHaveLength(1));
    await screen.findByRole("heading", { name: "Dashboard" });
    ipc.emit("navigate", "/settings");
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
  });

  it("ignores routes that are not on the allow-list", async () => {
    ipc.on("take_pending_route", "/definitely-not-a-page");
    render(<App />);
    await screen.findByRole("heading", { name: "Dashboard" });
    ipc.emit("navigate", "https://evil.example/");
    ipc.emit("navigate", 42);
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
  });
});
