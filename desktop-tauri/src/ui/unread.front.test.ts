// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref } from "vue";
import type { AgentPort } from "../port/port";
import type { RuntimeView, TreeWorkspace } from "../port/hub";
import { useViewed } from "./unread";
import { sessionKey, useUnreadSince } from "../state/newAnswers";

const rt: RuntimeView = { id: "r1", base: "", root: "/w", name: "w", sessionPath: "/a" };
const book = (unread: boolean): TreeWorkspace[] => [{ root: "/w", name: "w", sessions: [{ path: "/a", name: "a", ...(unread ? { unread: true } : {}) }] }];
const settle = () => new Promise((r) => setTimeout(r, 0));

function mount(initial: { tree: TreeWorkspace[]; rts: RuntimeView[]; focused?: boolean }) {
  vi.spyOn(document, "hasFocus").mockReturnValue(initial.focused ?? true);
  const port = { markSessionViewed: vi.fn(() => Promise.resolve()) } as unknown as AgentPort & { markSessionViewed: ReturnType<typeof vi.fn> };
  const ports = new Map([["r1", port as AgentPort]]);
  const tree = ref(initial.tree);
  const runtimes = ref(initial.rts);
  let hook!: ReturnType<typeof useViewed>;
  const utils = render(defineComponent({
    setup() {
      hook = useViewed({ runtimes, ports, active: "r1", tree, remote: {}, reload: () => Promise.resolve() });
      return () => h("div");
    },
  }));
  return { port, hook, tree, runtimes, ...utils };
}

function sinceOf(key: string) {
  let since!: ReturnType<typeof useUnreadSince>;
  render(defineComponent({
    setup() {
      since = useUnreadSince(key);
      return () => h("div");
    },
  }));
  return since;
}

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("a session that is already in front", () => {
  it("is marked seen when the window opens on it", async () => {
    const { port } = mount({ tree: book(true), rts: [rt] });
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });

  it("is marked seen when the session list arrives after the pane did", async () => {
    const { port, tree } = mount({ tree: [], rts: [rt] });
    await settle();
    expect(port.markSessionViewed).not.toHaveBeenCalled();
    tree.value = book(true);
    await nextTick();
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });

  it("is marked seen when the pane learns its session path after it was in front", async () => {
    const { port, runtimes } = mount({ tree: book(true), rts: [{ ...rt, sessionPath: "" }] });
    await settle();
    expect(port.markSessionViewed).not.toHaveBeenCalled();
    runtimes.value = [rt];
    await nextTick();
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });

  it("stays unread while the window has no focus, and is marked when focus returns", async () => {
    const { port } = mount({ tree: book(true), rts: [rt], focused: false });
    await settle();
    expect(port.markSessionViewed).not.toHaveBeenCalled();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });

  it("is not asked about again and again when the kernel refuses", async () => {
    const { port, tree } = mount({ tree: book(true), rts: [rt] });
    port.markSessionViewed.mockImplementation(() => Promise.reject(new Error("refused")));
    await settle();
    for (let i = 0; i < 4; i++) {
      tree.value = book(true);
      await nextTick();
      await settle();
    }
    expect(port.markSessionViewed.mock.calls.length).toBeLessThanOrEqual(2);
  });
});

describe("where the person last looked is kept for the new-answer line", () => {
  const row = (path: string, over: object): TreeWorkspace[] => [{ root: "/w", name: "w", sessions: [{ path, name: path, ...over }] }];

  it("is taken from the unread row the moment it is marked seen, and kept after the mark clears", async () => {
    const own = { ...rt, sessionPath: "/since-a" };
    const { tree } = mount({ tree: row("/since-a", { unread: true, viewedAt: 1234 }), rts: [own] });
    const since = sinceOf(sessionKey(undefined, "/since-a"));
    await settle();
    expect(since.value).toBe(1234);
    tree.value = row("/since-a", {});
    await nextTick();
    await settle();
    expect(since.value).toBe(1234);
  });

  it("is not taken for a row that is not unread, or from a kernel that sends no time", async () => {
    mount({ tree: row("/since-b", { viewedAt: 99 }), rts: [{ ...rt, sessionPath: "/since-b" }] });
    await settle();
    expect(sinceOf(sessionKey(undefined, "/since-b")).value).toBeUndefined();
    mount({ tree: row("/since-c", { unread: true }), rts: [{ ...rt, sessionPath: "/since-c" }] });
    await settle();
    expect(sinceOf(sessionKey(undefined, "/since-c")).value).toBeUndefined();
  });

  it("is taken when the list reports the row unread after the pane was already in front", async () => {
    const own = { ...rt, sessionPath: "/since-d" };
    const { tree } = mount({ tree: row("/since-d", {}), rts: [own] });
    await settle();
    tree.value = row("/since-d", { unread: true, viewedAt: 55 });
    await nextTick();
    await settle();
    expect(sinceOf(sessionKey(undefined, "/since-d")).value).toBe(55);
  });
});