// The kernel port: the Vue app's only route to the kernel. Every request
// goes through the Tauri IPC bridge (`invoke`), where the Rust shell holds
// the handshake token and adds the credential itself — the page composes
// paths and bodies, never secrets. The bridge is resolved per call: the
// shell injects __TAURI_INTERNALS__ after this module has loaded. In vitest
// (no Tauri host) the missing bridge rejects with the command named, which
// keeps stores and components testable without a kernel.
type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  const bridge = (globalThis as { __TAURI_INTERNALS__?: { invoke: Invoke } }).__TAURI_INTERNALS__
    ?.invoke;
  if (!bridge) {
    throw new Error(`no Tauri host: ${cmd} cannot run outside the shell`);
  }
  return bridge(cmd, args);
}

export interface KernelPort {
  connected(): Promise<boolean>;
  get<T>(path: string): Promise<T>;
  post<T>(path: string, body: unknown): Promise<T>;
}

export const kernelPort: KernelPort = {
  connected: async () => (await invoke("kernel_connected") as { connected: boolean }).connected,
  get: async <T>(path: string) => (await invoke("kernel_get", { path })) as T,
  post: async <T>(path: string, body: unknown) =>
    (await invoke("kernel_post", { path, body: JSON.stringify(body) })) as T,
};
