import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? sources(p) : /\.(ts|tsx)$/.test(n) ? [p] : [];
  });
}

const files = sources(join(import.meta.dirname, "..", "src"));

describe("frontend source hygiene", () => {
  it("finds the source files (guards against this test silently checking nothing)", () => {
    expect(files.length).toBeGreaterThan(30);
  });

  it("never injects HTML or evaluates strings", () => {
    const banned = [/dangerouslySetInnerHTML/, /\.innerHTML\s*=/, /\beval\s*\(/, /new Function\s*\(/, /document\.write\s*\(/, /insertAdjacentHTML/];
    const hits = files.flatMap((f) => {
      const text = readFileSync(f, "utf8");
      return banned.filter((re) => re.test(text)).map((re) => `${f}: ${re}`);
    });
    expect(hits).toEqual([]);
  });

  it("only talks to Rust through lib/ipc and the event listener, never the network", () => {
    const hits = files
      .filter((f) => !f.endsWith(join("lib", "ipc.ts")))
      .flatMap((f) => {
        const text = readFileSync(f, "utf8");
        return [/\bfetch\s*\(/, /XMLHttpRequest/, /WebSocket\s*\(/, /navigator\.sendBeacon/].filter((re) => re.test(text)).map((re) => `${f}: ${re}`);
      });
    expect(hits).toEqual([]);
  });

  it("never stores an API key in browser storage", () => {
    const hits = files.filter((f) => /localStorage\.setItem\([^)]*\b(api_?key|secret|token|password)\b/i.test(readFileSync(f, "utf8")));
    expect(hits).toEqual([]);
  });
});
