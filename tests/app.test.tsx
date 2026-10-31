import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "@/app/App";
import { NAV_ITEMS } from "@/app/navigation";

// Outside Tauri there is no event bridge; the hook must degrade gracefully.
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.reject(new Error("not in tauri")),
}));

describe("app shell", () => {
  it("lists all main navigation entries in spec order", () => {
    expect(NAV_ITEMS.map((i) => i.label)).toEqual([
      "Dashboard",
      "Processes",
      "Storage",
      "Diagnosis",
      "Actions",
      "History",
      "Settings",
    ]);
  });

  it("renders dashboard by default and navigates via sidebar", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();

    await user.click(screen.getByRole("link", { name: "Storage" }));
    expect(screen.getByRole("heading", { name: "Storage" })).toBeInTheDocument();
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

  it("falls back to defaults on corrupt stored settings", async () => {
    localStorage.setItem("devpilot.settings.v1", "{not json");
    render(<App />);
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
