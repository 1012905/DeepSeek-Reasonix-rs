// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick, ref, type MaybeRefOrGetter } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { useAgentRequest } from "./useAgentRequest";
import type { AgentPort } from "../port/port";
import type { SubagentRun } from "../port/subagent";

const runs: SubagentRun[] = [
  { ref: "sa_1", name: "audit", kind: "task", model: "m", state: "completed", parentCall: "call_a", startedAt: 1, updatedAt: 2 },
  { ref: "sa_2", name: "review", kind: "task", model: "m", state: "completed", parentCall: "call_b", startedAt: 1, updatedAt: 2 },
  { ref: "sa_3", name: "review", kind: "task", model: "m", state: "completed", parentCall: "call_b", startedAt: 1, updatedAt: 2 },
];

const listing = (over: { subagents?: () => Promise<SubagentRun[]> } = {}): AgentPort =>
  ({ subagents: vi.fn(over.subagents ?? (async () => runs)) } as unknown as AgentPort);

const mounted: VueWrapper[] = [];
afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount();
});

function harness(port: MaybeRefOrGetter<AgentPort>) {
  const reveal = vi.fn();
  let hook!: ReturnType<typeof useAgentRequest>;
  const wrapper = mount(defineComponent({
    setup() {
      hook = useAgentRequest(port, reveal);
      return () => h("div");
    },
  }));
  mounted.push(wrapper);
  return { hook, reveal };
}

describe("opening a task card's sub-agents in the workbench", () => {
  it("opens the run the card's call spawned and reveals the workbench", async () => {
    const { hook, reveal } = harness(listing());
    const opened = await hook.open("call_a");
    expect(opened).toBe(true);
    expect(hook.request.value?.runs).toEqual([{ ref: "sa_1", name: "audit" }]);
    expect(reveal).toHaveBeenCalledTimes(1);
  });

  it("opens every run of a call that spawned several", async () => {
    const { hook } = harness(listing());
    expect(await hook.open("call_b")).toBe(true);
    expect(hook.request.value?.runs).toEqual([{ ref: "sa_2", name: "review" }, { ref: "sa_3", name: "review" }]);
  });

  it("reports false for a call with no run, and opens nothing", async () => {
    const { hook, reveal } = harness(listing());
    expect(await hook.open("call_none")).toBe(false);
    expect(hook.request.value).toBeUndefined();
    expect(reveal).not.toHaveBeenCalled();
  });

  it("reports false when the kernel cannot list runs", async () => {
    const { hook, reveal } = harness(listing({ subagents: async () => { throw new Error("404"); } }));
    expect(await hook.open("call_a")).toBe(false);
    expect(hook.request.value).toBeUndefined();
    expect(reveal).not.toHaveBeenCalled();
  });

  it("opens one background job's output as a jobs request", () => {
    const { hook, reveal } = harness(listing());
    hook.openJob({ id: "job_1", name: "build" });
    expect(hook.request.value?.jobs).toEqual([{ id: "job_1", name: "build" }]);
    expect(reveal).toHaveBeenCalledTimes(1);
  });

  it("clears the request once the workbench reports it taken", async () => {
    const { hook } = harness(listing());
    await hook.open("call_a");
    const req = hook.request.value;
    expect(req).toBeDefined();
    req!.consumed!();
    expect(hook.request.value).toBeUndefined();
  });

  it("a superseded request's consumed does not clear the current one", async () => {
    const { hook } = harness(listing());
    await hook.open("call_a");
    const first = hook.request.value!;
    await hook.open("call_b");
    first.consumed!();
    expect(hook.request.value?.runs).toEqual([{ ref: "sa_2", name: "review" }, { ref: "sa_3", name: "review" }]);
  });

  it("clears the request when the pane's port changes", async () => {
    const port = ref(listing());
    const { hook } = harness(port);
    await hook.open("call_a");
    expect(hook.request.value).toBeDefined();
    port.value = listing();
    await nextTick();
    expect(hook.request.value).toBeUndefined();
  });
});