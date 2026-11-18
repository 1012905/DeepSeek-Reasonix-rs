// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref } from "vue";
import "./testkit";
import { usePinnedSessions } from "./usePinnedSessions";
import type { HubPort, TreeWorkspace } from "../port/hub";

const KEY = "reasonix:pinned-sessions";
beforeEach(() => localStorage.clear());
afterEach(cleanup);

const rows = (...s: { path: string; pinned?: boolean }[]) =>
  [{ root: "/w", name: "w", sessions: s.map((x) => ({ name: x.path, ...x })) }] as unknown as TreeWorkspace[];
const hubOf = (over: Partial<Record<"syncPins" | "pinSession", unknown>> = {}) =>
  ({ syncPins: vi.fn(async (_p: string[]) => {}), pinSession: vi.fn(async () => {}), ...over }) as unknown as HubPort & {
    syncPins: ReturnType<typeof vi.fn>;
    pinSession: ReturnType<typeof vi.fn>;
  };

function mount(hub: HubPort, t: TreeWorkspace[], read: boolean, fail?: (e: unknown) => void) {
  const tree = ref(t);
  const treeRead = ref(read);
  let hook!: ReturnType<typeof usePinnedSessions>;
  const utils = render(defineComponent({
    setup() {
      hook = usePinnedSessions(hub, tree, treeRead, fail);
      return () => h("div");
    },
  }));
  const setTree = async (next: TreeWorkspace[]) => { tree.value = next; await nextTick(); };
  const setRead = async (next: boolean) => { treeRead.value = next; await nextTick(); };
  return { hook, setTree, setRead, ...utils };
}

describe("what is drawn as pinned", () => {
  it("is what the kernel reports, not what this window remembers", () => {
    localStorage.setItem(KEY, JSON.stringify(["/gone.jsonl"]));
    const { hook } = mount(hubOf(), rows({ path: "/k.jsonl", pinned: true }, { path: "/gone.jsonl" }), true);
    expect([...hook[0].value]).toEqual(["/k.jsonl"]);
  });

  it("follows another window unpinning, and does not pin it back", async () => {
    const hub = hubOf();
    const { hook, setTree } = mount(hub, rows({ path: "/k.jsonl", pinned: true }), true);
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalledTimes(1));
    await setTree(rows({ path: "/k.jsonl" }));
    expect(hook[0].value.has("/k.jsonl")).toBe(false);
    await new Promise((r) => setTimeout(r, 30));
    expect(hub.syncPins).toHaveBeenCalledTimes(1);
  });

  it("unpin, reload, archive does not bring the pin back", async () => {
    const hub = hubOf();
    const { hook, setTree } = mount(hub, rows({ path: "/k.jsonl", pinned: true }), true);
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalled());
    hook[2]("/k.jsonl");
    expect(hook[0].value.has("/k.jsonl")).toBe(false);
    await setTree(rows({ path: "/k.jsonl", pinned: true }));
    expect(hook[0].value.has("/k.jsonl")).toBe(false);
    await setTree(rows({ path: "/k.jsonl" }));
    expect(hook[0].value.has("/k.jsonl")).toBe(false);
    expect(hub.syncPins).toHaveBeenCalledTimes(1);
  });
});

describe("toggling", () => {
  it("shows a pin at once and keeps it until the tree agrees", async () => {
    const hub = hubOf();
    const { hook, setTree } = mount(hub, rows({ path: "/x.jsonl" }), true);
    void hook[1]("/x.jsonl");
    expect(hub.pinSession).toHaveBeenCalledWith("/x.jsonl", true);
    expect(hook[0].value.has("/x.jsonl")).toBe(true);
    await setTree(rows({ path: "/x.jsonl", pinned: true }));
    expect(hook[0].value.has("/x.jsonl")).toBe(true);
  });

  it("withdraws a pin the kernel refused and reports it", async () => {
    const hub = hubOf({ pinSession: vi.fn().mockRejectedValue(new Error("down")) });
    const fail = vi.fn();
    const { hook } = mount(hub, rows({ path: "/x.jsonl" }), true, fail);
    void hook[1]("/x.jsonl");
    await waitFor(() => expect(fail).toHaveBeenCalledTimes(1));
    expect(hook[0].value.has("/x.jsonl")).toBe(false);
  });

  it("keeps a pin the kernel refused to drop", async () => {
    const hub = hubOf({ pinSession: vi.fn().mockRejectedValue(new Error("down")) });
    const { hook } = mount(hub, rows({ path: "/k.jsonl", pinned: true }), true);
    void hook[1]("/k.jsonl");
    await waitFor(() => expect(hub.pinSession).toHaveBeenCalled());
    await waitFor(() => expect(hook[0].value.has("/k.jsonl")).toBe(true));
  });
});

describe("pins from before the kernel kept them", () => {
  it("opens the gate with nothing to send once the tree has loaded", async () => {
    const hub = hubOf();
    const { setRead } = mount(hub, rows({ path: "/x.jsonl" }), false);
    expect(hub.syncPins).not.toHaveBeenCalled();
    await setRead(true);
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalledWith([]));
  });

  it("sends each once, when its conversation is first listed, then forgets it", async () => {
    localStorage.setItem(KEY, JSON.stringify(["/late.jsonl", "/x.jsonl"]));
    const hub = hubOf();
    const { hook, setTree } = mount(hub, rows({ path: "/x.jsonl" }), true);
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalledWith(["/x.jsonl"]));
    await waitFor(() => expect(hook[0].value.has("/x.jsonl")).toBe(true));
    expect(JSON.parse(localStorage.getItem(KEY) ?? "[]")).toEqual(["/late.jsonl"]);
    await setTree(rows({ path: "/x.jsonl", pinned: true }, { path: "/late.jsonl" }));
    await waitFor(() => expect(hub.syncPins).toHaveBeenLastCalledWith(["/late.jsonl"]));
    await waitFor(() => expect(localStorage.getItem(KEY)).toBeNull());
  });

  it("retries after a failed send and leaves the memory in place", async () => {
    localStorage.setItem(KEY, JSON.stringify(["/x.jsonl"]));
    const hub = hubOf({ syncPins: vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue(undefined) });
    const { setTree } = mount(hub, rows({ path: "/x.jsonl" }), true);
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalledTimes(1));
    expect(JSON.parse(localStorage.getItem(KEY) ?? "[]")).toEqual(["/x.jsonl"]);
    await setTree(rows({ path: "/x.jsonl" }, { path: "/y.jsonl" }));
    await waitFor(() => expect(hub.syncPins).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(localStorage.getItem(KEY)).toBeNull());
  });
});