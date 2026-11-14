// The Studio store's kernel traffic: the runtime boot opens on the shell's
// own working directory when none is open, /history is a read, /submit
// carries the input under the host's own field name, a submitted turn ends
// with a history refresh, and the live stream narrows frames to the fields
// the transcript renders — the port and the event bridge are stubbed, so the
// shapes are what's under test.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useStudioStore } from "./studio";

const get = vi.fn();
const post = vi.fn();
const defaultRoot = vi.fn();

vi.mock("../port/kernel", () => ({
  kernelPort: {
    get: (...a: unknown[]) => get(...(a as [string])),
    post: (...a: unknown[]) => post(...(a as [string, unknown])),
    connected: async () => true,
    defaultRoot: (...a: unknown[]) => defaultRoot(...(a as [])),
  },
}));

// The event bridge is stubbed at the same seam the Rust shell emits on; the
// captured handler lets a test drive frames through the store's narrowing.
let captured: ((e: { payload: string }) => void) | undefined;

vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_event: string, handler: (e: { payload: string }) => void) => {
    captured = handler;
    return () => {};
  },
}));

const history = [
  { role: "user", content: "hello", msgIndex: 0 },
  { role: "assistant", content: "hi", msgIndex: 1 },
];

function emitFrame(payload: string) {
  captured?.({ payload });
}

beforeEach(() => {
  setActivePinia(createPinia());
  get.mockReset();
  post.mockReset();
  defaultRoot.mockReset();
    get.mockResolvedValue(history);
});

describe("studio store", () => {
  it("refreshHistory reads the projected transcript", async () => {
    const store = useStudioStore();
    await store.refreshHistory();
    expect(get).toHaveBeenCalledWith("/history");
    expect(store.history).toEqual(history);
    expect(store.error).toBe("");
  });

  it("ensureRuntime opens on the shell's working directory when none is open", async () => {
    const store = useStudioStore();
    get.mockResolvedValueOnce([]);
    defaultRoot.mockResolvedValue("C:\\dev\\the-project");
    post.mockResolvedValueOnce({ id: "rt1", base: "/rt/rt1" });
    await store.ensureRuntime();
    expect(defaultRoot).toHaveBeenCalled();
    expect(post).toHaveBeenCalledWith("/runtimes", { root: "C:\\dev\\the-project" });
    expect(get).toHaveBeenCalledWith("/runtimes");
    expect(store.runtimeId).toBe("rt1");
  });

  it("ensureRuntime reuses the first open runtime", async () => {
    const store = useStudioStore();
    get.mockResolvedValueOnce([{ id: "rt-already", base: "/rt/rt-already" }]);
    await store.ensureRuntime();
    expect(post).not.toHaveBeenCalled();
    expect(store.runtimeId).toBe("rt-already");
  });

  it("submit posts the input under the host's field and refreshes", async () => {
    const store = useStudioStore();
    post.mockResolvedValue(undefined);
    await store.submit("add retry to the http client");
    expect(post).toHaveBeenCalledWith("/submit", { input: "add retry to the http client" });
    expect(get).toHaveBeenCalledWith("/history");
    expect(store.submitting).toBe(false);
  });

  it("submit ignores blank input without touching the kernel", async () => {
    const store = useStudioStore();
    await store.submit("   ");
    expect(post).not.toHaveBeenCalled();
  });

  it("a kernel error surfaces in the store, not as an exception", async () => {
    const store = useStudioStore();
    get.mockRejectedValueOnce(new Error("the kernel refused /history: 403"));
    await store.refreshHistory();
    expect(store.error).toContain("403");
    expect(store.history).toEqual([]);
  });

  it("the stream narrows frames and turn_done folds into the history", async () => {
    const store = useStudioStore();
    await store.startStream();
    emitFrame('{"kind":"text","text":"working on it"}');
    emitFrame('{"kind":"text","text":" done"}');
    emitFrame('{"kind":"turn_done"}');
    expect(store.live).toHaveLength(3);
    expect(store.live[0]).toEqual({ kind: "text", text: "working on it" });
    expect(store.live[2]).toEqual({ kind: "turn_done" });
    expect(get).toHaveBeenLastCalledWith("/history");
  });

  it("frames the kernel sends outside the contract disappear silently", async () => {
    const store = useStudioStore();
    await store.startStream();
    emitFrame('{"kind":"mystery_kind","text":"x"}');
    emitFrame("not even json");
    expect(store.live).toHaveLength(0);
  });
});
