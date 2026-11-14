// The port is the page's only route to the kernel, so its shapes are the
// contract the Rust commands read: kernel_get takes a bare path, kernel_post
// a path and a stringified body — the JSON crossing is the shell's to parse.
// Without a Tauri host every call rejects naming the command; nothing
// pretends to succeed.
import { afterEach, describe, expect, it, vi } from "vitest";
import { kernelPort } from "./kernel";

const internals = globalThis as { __TAURI_INTERNALS__?: { invoke: unknown } };

afterEach(() => {
  delete internals.__TAURI_INTERNALS__;
});

describe("kernelPort outside the shell", () => {
  it("refuses to run, naming the command", async () => {
    await expect(kernelPort.connected()).rejects.toThrow(/no Tauri host: kernel_connected/);
    await expect(kernelPort.get("/sessions")).rejects.toThrow(/no Tauri host: kernel_get/);
    await expect(kernelPort.post("/turns", { task: "x" })).rejects.toThrow(/no Tauri host: kernel_post/);
  });
});

describe("kernelPort against a stubbed bridge", () => {
  it("sends the shapes the Rust commands expect", async () => {
    const invoke = vi.fn(async (cmd: string, _args?: Record<string, unknown>) => {
      if (cmd === "kernel_connected") return { connected: true };
      if (cmd === "kernel_get") return [1, 2];
      return "ok";
    });
    internals.__TAURI_INTERNALS__ = { invoke };

    expect(await kernelPort.connected()).toBe(true);
    expect(await kernelPort.get<number[]>("/sessions")).toEqual([1, 2]);
    expect(await kernelPort.post("/turns", { task: "x" })).toBe("ok");
    expect(invoke.mock.calls).toEqual([
      ["kernel_connected", undefined],
      ["kernel_get", { path: "/sessions" }],
      ["kernel_post", { path: "/turns", body: '{"task":"x"}' }],
    ]);
  });
});
