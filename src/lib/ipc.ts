import { invoke } from "@tauri-apps/api/core";
import type { CodedError } from "@/types/api";

/** Turn whatever Rust (or a missing bridge) throws into one readable message. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object") {
    const o = e as Partial<CodedError> & { message?: unknown };
    if (typeof o.message === "string" && o.message) return o.message;
  }
  return "Something went wrong.";
}

export function errorCode(e: unknown): string | undefined {
  if (e && typeof e === "object" && "code" in e && typeof (e as CodedError).code === "string") {
    return (e as CodedError).code;
  }
  return undefined;
}

/**
 * Typed command call. The frontend can only reach commands Rust registered; there is no shell,
 * file, or arbitrary-process access from here.
 */
export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}
