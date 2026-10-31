import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "@/app/App";
import { NAV_ITEMS } from "@/app/navigation";

describe("app shell", () => {
  it("lists all main navigation entries in spec order", () => {
    expect(NAV_ITEMS.map((i) => i.label)).toEqual(["Dashboard", "Processes", "Storage", "Diagnosis", "Actions", "History", "Settings"]);
  });

  it("renders the dashboard by default and navigates via the sidebar", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "Storage" }));
    expect(screen.getByRole("heading", { name: "Storage" })).toBeInTheDocument();
  });

  it("keeps working when the backend is unreachable", async () => {
    // No handlers registered: every command rejects. Pages must show a message, not crash.
    render(<App />);
    expect(await screen.findByText(/Waiting for the first/i)).toBeInTheDocument();
  });

  it("loads the History page lazily", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(within(screen.getByRole("navigation", { name: "Main" })).getByRole("link", { name: "History" }));
    expect(await screen.findByRole("heading", { name: "History" })).toBeInTheDocument();
  });
});

describe("settings", () => {
  it("switches to dark theme and persists it", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("link", { name: "Settings" }));
    await user.click(screen.getByRole("button", { name: "Dark" }));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(JSON.parse(localStorage.getItem("devpilot.settings.v1")!).theme).toBe("dark");
  });

  it("falls back to defaults on corrupt stored settings", () => {
    localStorage.setItem("devpilot.settings.v1", "{not json");
    render(<App />);
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("merges older stored settings with new defaults", async () => {
    localStorage.setItem("devpilot.settings.v1", JSON.stringify({ theme: "dark", monitorIntervalSec: 5 }));
    const user = userEvent.setup();
    render(<App />);
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    await user.click(screen.getByRole("link", { name: "Settings" }));
    // New fields exist with safe defaults: AI is off, automation is off.
    expect(screen.getByRole("switch", { name: "Enable AI assistance" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("switch", { name: "Automatically perform safe optimizations" })).toHaveAttribute("aria-checked", "false");
  });
});
