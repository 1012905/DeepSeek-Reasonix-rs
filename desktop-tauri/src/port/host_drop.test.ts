import { afterEach, describe, expect, it, vi } from "vitest";

// The drop contract has two shapes and both must keep working: Electron's
// synchronous pathForFile, and a shell whose paths arrive only as a native
// event. A bridge missing either member degrades, never throws.

async function hostOver(win: Record<string, unknown>) {
  vi.resetModules();
  vi.stubGlobal("window", win);
  const { host } = await import("./host");
  return host();
}

afterEach(() => vi.unstubAllGlobals());

describe("drop paths", () => {
  it("resolves through pathForFile where the bridge has it", async () => {
    const port = await hostOver({
      reasonixHost: {
        shell: "electron",
        platform: "darwin",
        titleBar: false,
        pathForFile: (f: File) => (f.name === "a.txt" ? "/w/a.txt" : ""),
      },
    });
    const files = [new File(["x"], "a.txt"), new File(["y"], "b.txt")];
    expect(port.pathsForFiles(files)).toEqual(["/w/a.txt"]);
  });

  it("answers no paths where the bridge cannot place files", async () => {
    const port = await hostOver({
      reasonixHost: { shell: "tauri", platform: "windows", titleBar: true },
    });
    expect(port.pathsForFiles([new File(["x"], "a.txt")])).toEqual([]);
  });

  it("forwards the native event subscription and its teardown", async () => {
    const held: { listener: ((paths: string[]) => void) | null } = { listener: null };
    let torn = false;
    const port = await hostOver({
      reasonixHost: {
        shell: "tauri",
        platform: "windows",
        titleBar: true,
        onDropPaths: (fn: (paths: string[]) => void) => {
          held.listener = fn;
          return () => {
            torn = true;
          };
        },
      },
    });
    const seen: string[][] = [];
    const off = port.onDropPaths((paths) => seen.push(paths));
    held.listener?.(["/w/a.txt"]);
    expect(seen).toEqual([["/w/a.txt"]]);
    off();
    expect(torn).toBe(true);
  });

  it("subscribes to nothing where the shell has no drop event", async () => {
    const port = await hostOver({
      reasonixHost: { shell: "electron", platform: "linux", titleBar: false },
    });
    expect(() => port.onDropPaths(() => {})()).not.toThrow();
  });

  it("is a no-op in a browser tab", async () => {
    const port = await hostOver({});
    expect(port.pathsForFiles([new File(["x"], "a.txt")])).toEqual([]);
    expect(() => port.onDropPaths(() => {})()).not.toThrow();
  });
});