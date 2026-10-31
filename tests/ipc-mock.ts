/**
 * In-memory stand-in for the Tauri bridge. Tests register handlers per command and can emit events,
 * so UI behavior is exercised without a Rust process.
 */
type Handler = (args: Record<string, unknown> | undefined) => unknown;

const handlers = new Map<string, Handler>();
const listeners = new Map<string, Set<(e: { payload: unknown }) => void>>();
export const calls: { cmd: string; args?: Record<string, unknown> }[] = [];

export const ipc = {
  reset() {
    handlers.clear();
    listeners.clear();
    calls.length = 0;
  },
  /** Register a handler. Returning a value resolves; throwing rejects with the thrown value. */
  on(cmd: string, handler: Handler | unknown) {
    handlers.set(cmd, typeof handler === "function" ? (handler as Handler) : () => handler);
  },
  invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
    calls.push({ cmd, args });
    const h = handlers.get(cmd);
    if (!h) return Promise.reject(`no mock handler for ${cmd}`);
    try {
      return Promise.resolve(h(args));
    } catch (e) {
      return Promise.reject(e);
    }
  },
  listen(event: string, cb: (e: { payload: unknown }) => void): Promise<() => void> {
    const set = listeners.get(event) ?? new Set();
    set.add(cb);
    listeners.set(event, set);
    return Promise.resolve(() => set.delete(cb));
  },
  emit(event: string, payload: unknown) {
    listeners.get(event)?.forEach((cb) => cb({ payload }));
  },
  calledWith(cmd: string) {
    return calls.filter((c) => c.cmd === cmd);
  },
};
