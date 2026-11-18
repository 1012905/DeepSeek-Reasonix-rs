// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref } from "vue";
import type { AgentPort } from "../port/port";
import type { RuntimeView, TreeWorkspace } from "../port/hub";
import { useViewed } from "./unread";

const rt: RuntimeView = { id: "r1", base: "", root: "/w", name: "w", sessionPath: "/a" };
const book = (unread: boolean): TreeWorkspace[] => [{ root: "/w", name: "w", sessions: [{ path: "/a", name: "a", ...(unread ? { unread: true } : {}) }] }];
const settle = () => new Promise((r) => setTimeout(r, 0));

function setup(call: () => Promise<void>, reload: () => Promise<unknown>, rts: RuntimeView[] = [rt]) {
  const port = { markSessionViewed: vi.fn(call) } as unknown as AgentPort & { markSessionViewed: ReturnType<typeof vi.fn> };
  const ports = new Map([["r1", port as AgentPort]]);
  const tree = ref(book(true));
  const runtimes = ref(rts);
  let hook!: ReturnType<typeof useViewed>;
  const utils = render(defineComponent({
    setup() {
      hook = useViewed({ runtimes, ports, active: "r1", tree, remote: {}, reload });
      return () => h("div");
    },
  }));
  return { port, hook, tree, runtimes, ...utils };
}

beforeEach(() => {
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("useViewed", () => {
  it("keeps the optimistic clear when the reload after a successful call fails, until the tree next changes", async () => {
    const { hook, tree } = setup(() => Promise.resolve(), () => Promise.reject(new Error("offline")));
    hook.view("r1");
    await settle();
    expect(hook.tree.value[0].sessions[0].unread).toBe(false);
    tree.value = book(true);
    await nextTick();
    expect(hook.tree.value[0].sessions[0].unread).toBe(true);
  });

  it("restores the mark when the call itself is refused", async () => {
    const { hook } = setup(() => Promise.reject(new Error("refused")), () => Promise.resolve());
    hook.view("r1");
    await settle();
    expect(hook.tree.value[0].sessions[0].unread).toBe(true);
  });

  it("owes one more call for a turn that ends while the first is still in flight", async () => {
    let release: () => void = () => {};
    const { port, hook } = setup(() => new Promise<void>((r) => (release = r)), () => Promise.resolve());
    hook.view("r1");
    await settle();
    hook.turnDone("r1");
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
    release();
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(2);
  });

  it("drops an owed call when the pane has moved to another session by the time the first finishes", async () => {
    let release: () => void = () => {};
    const { port, hook, tree, runtimes } = setup(() => new Promise<void>((r) => (release = r)), () => Promise.resolve());
    hook.view("r1");
    await settle();
    hook.turnDone("r1");
    tree.value = book(true);
    runtimes.value = [{ ...rt, sessionPath: "/b" }];
    await nextTick();
    release();
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });

  it("does not repeat when nothing ended during the call", async () => {
    const { port, hook } = setup(() => Promise.resolve(), () => Promise.resolve());
    hook.view("r1");
    await settle();
    expect(port.markSessionViewed).toHaveBeenCalledTimes(1);
  });
});