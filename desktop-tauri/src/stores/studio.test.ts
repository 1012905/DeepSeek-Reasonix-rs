// The Studio store's kernel traffic: /history is a read, /submit carries the
// input under the host's own field name, and a submitted turn ends with a
// history refresh — the port is stubbed, so the shapes are what's under test.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useStudioStore } from "./studio";

const get = vi.fn();
const post = vi.fn();

vi.mock("../port/kernel", () => ({
  kernelPort: {
    get: (...a: unknown[]) => get(...(a as [string])),
    post: (...a: unknown[]) => post(...(a as [string, unknown])),
    connected: async () => true,
  },
}));

const history = [
  { role: "user", content: "hello", msgIndex: 0 },
  { role: "assistant", content: "hi", msgIndex: 1 },
];

beforeEach(() => {
  setActivePinia(createPinia());
  get.mockReset();
  post.mockReset();
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
});
