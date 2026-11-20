// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

// The window wires two channels into one router: the DOM drop and the shell's
// native path event. Either order settles the drop once, and a shell with only
// the synchronous answer keeps working.

interface Bridge {
  pathForFile?: (f: File) => string;
  onDropPaths?: (fn: (paths: string[]) => void) => () => void;
}

let native: ((paths: string[]) => void) | null = null;

async function installOver(bridge: Bridge) {
  vi.resetModules();
  native = null;
  (window as unknown as { reasonixHost: unknown }).reasonixHost = {
    shell: "tauri",
    platform: "windows",
    titleBar: true,
    ...bridge,
    onDropPaths: (fn: (paths: string[]) => void) => {
      native = fn;
      return () => {};
    },
  };
  const { install, useFileDrop } = await import("./filedrop");
  install();
  const el = document.createElement("div");
  document.body.appendChild(el);
  const drops: { paths: string[]; files: File[] }[] = [];
  const off = useFileDrop(
    (d) => drops.push({ paths: d.paths, files: d.files }),
    () => {},
  )(el);
  return { el, drops, off };
}

function drag(el: HTMLElement, type: string, files: File[] = []) {
  const e = new Event(type, { bubbles: true, cancelable: true });
  Object.assign(e, {
    dataTransfer: { types: ["Files"], files, getData: () => "" },
    relatedTarget: null,
  });
  el.dispatchEvent(e);
}

afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
  delete (window as unknown as { reasonixHost?: unknown }).reasonixHost;
});

describe("the native path channel", () => {
  it("settles a DOM drop that carries no paths of its own", async () => {
    vi.useFakeTimers();
    const { el, drops, off } = await installOver({});
    const file = new File(["x"], "a.txt");
    drag(el, "drop", [file]);
    expect(drops).toEqual([]);
    native?.(["/w/a.txt"]);
    expect(drops).toEqual([{ paths: ["/w/a.txt"], files: [file] }]);
    vi.advanceTimersByTime(500);
    expect(drops.length).toBe(1);
    off();
  });

  it("settles once when the paths beat the DOM drop to it", async () => {
    vi.useFakeTimers();
    const { el, drops, off } = await installOver({});
    const file = new File(["x"], "a.txt");
    drag(el, "dragover", [file]);
    native?.(["/w/a.txt"]);
    expect(drops).toEqual([{ paths: ["/w/a.txt"], files: [] }]);
    drag(el, "drop", [file]);
    vi.advanceTimersByTime(500);
    expect(drops.length).toBe(1);
    off();
  });

  it("still resolves through a synchronous pathForFile", async () => {
    vi.useFakeTimers();
    const { el, drops, off } = await installOver({ pathForFile: () => "/w/a.txt" });
    drag(el, "drop", [new File(["x"], "a.txt")]);
    expect(drops).toEqual([{ paths: ["/w/a.txt"], files: [expect.any(File)] }]);
    off();
  });
});