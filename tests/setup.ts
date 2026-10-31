import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { ipc } from "./ipc-mock";

// Every test talks to the in-memory bridge, never to a real Tauri runtime.
vi.mock("@tauri-apps/api/core", () => ({ invoke: ipc.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: ipc.listen }));

// jsdom-family environments have no matchMedia; settings/theme code needs it.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
    onchange: null,
  }),
});

beforeEach(() => {
  ipc.reset();
  localStorage.clear();
  window.location.hash = "#/";
  document.documentElement.classList.remove("dark");
});

afterEach(() => cleanup());
