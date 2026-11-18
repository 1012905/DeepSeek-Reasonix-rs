// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import "./testkit";
import AgentRunView from "./AgentRunView.vue";
import { boot, t } from "../i18n";
import type { AgentPort, HistoryMessage } from "../port/port";
import type { SubagentRun, SubagentTranscript } from "../port/subagent";

beforeEach(() => boot());

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  vi.useRealTimers();
});

const run = (over: Partial<SubagentRun> = {}): SubagentRun =>
  ({ ref: "sa_1", name: "audit", kind: "task", model: "deepseek/pro", state: "running", startedAt: 1, updatedAt: 2, ...over });

const say = (content: string, msgIndex: number): HistoryMessage => ({ role: "assistant", content, msgIndex });
const asked = (content: string): HistoryMessage => ({ role: "user", content, msgIndex: 0 });

function reader(pages: SubagentTranscript[]) {
  const calls: number[] = [];
  const port = {
    workspaceImageURL: () => "",
    subagentTranscript: vi.fn(async (_ref: string, after = 0) => {
      calls.push(after);
      return pages.find((p) => p.after === after) ?? pages[pages.length - 1];
    }),
  } as unknown as AgentPort;
  return { port, calls };
}

const page = (messages: HistoryMessage[], after: number, over: Partial<SubagentTranscript> = {}): SubagentTranscript => ({
  run: run(), messages, after, next: after + messages.length, total: after + messages.length, live: true, ...over,
});

function draw(port: AgentPort, over: { shown?: boolean; onShowCall?: (callId: string) => void } = {}) {
  const wrapper = mount(AgentRunView, {
    props: { port, run: { ref: "sa_1", name: "audit" }, shown: true, ...over },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return wrapper;
}

describe("a sub-agent run read from the kernel's record", () => {
  it("shows what the run said and the state the kernel reports", async () => {
    const { port } = reader([page([say("reading the schema", 1)], 0, { live: false, run: run({ state: "completed" }) })]);
    const w = draw(port);
    await flushPromises();
    expect(w.text()).toContain("reading the schema");
    expect(w.get(".rt").text()).toBe(t("common.done_2"));
    expect(w.get(".model").text()).toBe("deepseek/pro");
    expect(w.get(".agentrun-note").text()).toContain(t("subagent.tab.read_only"));
    expect(w.find('[data-action="agent.show-call"]').exists()).toBe(false);
  });

  it("keeps polling from where the last page ended while the run is live, and stops once it is not", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { port, calls } = reader([
      page([say("first", 1)], 0),
      page([say("second", 2)], 1),
      page([], 2, { live: false, run: run({ state: "completed" }) }),
    ]);
    const w = draw(port);
    await flushPromises();
    expect(w.text()).toContain("first");
    await vi.advanceTimersByTimeAsync(1600);
    expect(w.text()).toContain("second");
    expect(w.text()).toContain("first");
    await vi.advanceTimersByTimeAsync(1600);
    const settled = calls.length;
    await vi.advanceTimersByTimeAsync(6000);
    expect(calls.slice(0, 3)).toEqual([0, 1, 2]);
    expect(calls.length).toBe(settled);
  });

  it("reads further pages straight away when the run holds more than one page", async () => {
    const { port, calls } = reader([
      page([say("one", 1)], 0, { total: 2 }),
      page([say("two", 2)], 1, { live: false, run: run({ state: "completed" }) }),
    ]);
    const w = draw(port);
    await flushPromises();
    expect(w.text()).toContain("two");
    expect(calls).toEqual([0, 1]);
  });

  it("says why it cannot be read instead of staying empty", async () => {
    const port = { workspaceImageURL: () => "", subagentTranscript: vi.fn(async () => { throw new Error("no such run"); }) } as unknown as AgentPort;
    const w = draw(port);
    await flushPromises();
    expect(w.get('[role="alert"]').text()).toContain("no such run");
    expect(w.find(".workbench-empty").exists()).toBe(false);
  });

  it("reads nothing while its tab is not shown", async () => {
    const { port } = reader([page([say("x", 1)], 0)]);
    const w = draw(port, { shown: false });
    await flushPromises();
    expect(port.subagentTranscript).not.toHaveBeenCalled();
    expect(w.find(".workbench-empty").exists()).toBe(true);
  });
});

describe("the sub-agent tab's frame", () => {
  it("puts the task the main session handed over above the run, once", async () => {
    const taskText = "给 export 补测试，只改 tests/";
    const { port } = reader([page([asked(taskText), say("reading the schema", 1)], 0, { live: false, run: run({ state: "completed" }) })]);
    const w = draw(port);
    await flushPromises();
    expect(w.text()).toContain("reading the schema");
    expect(w.get(".agentrun-task").text()).toContain(taskText);
    expect(w.get(".agentrun-task b").text()).toBe(t("subagent.tab.task"));
    expect(w.text().split(taskText)).toHaveLength(2);
  });

  it("stops the run through the call that spawned it while it runs, and only then", async () => {
    const { port } = reader([page([say("working", 1)], 0, { live: false, run: run({ parentCall: "call-9" }) })]);
    const stopToolCall = vi.fn(async () => {});
    Object.assign(port, { stopToolCall });
    const w = draw(port);
    await flushPromises();
    const stop = w.get('[data-action="agent.stop"]');
    expect(stop.text()).toBe(t("subagent.tab.stop"));
    await stop.trigger("click");
    expect(stopToolCall).toHaveBeenCalledWith("call-9");
    w.unmount();
    opened.pop();
    const done = reader([page([say("done", 1)], 0, { live: false, run: run({ parentCall: "call-9", state: "completed" }) })]);
    const w2 = draw(done.port);
    await flushPromises();
    expect(w2.text()).toContain("done");
    expect(w2.find('[data-action="agent.stop"]').exists()).toBe(false);
    expect(w2.get(".rt").text()).toBe(t("common.done_2"));
  });

  it("shows the state, not a stop button, when the window cannot stop a call", async () => {
    const { port } = reader([page([say("working", 1)], 0, { live: false, run: run({ parentCall: "call-9" }) })]);
    const w = draw(port);
    await flushPromises();
    expect(w.find('[data-action="agent.stop"]').exists()).toBe(false);
    expect(w.get(".rt").text()).toBe(t("common.running"));
  });

  it("goes back to the group in the conversation by the spawning call", async () => {
    const { port } = reader([page([say("working", 1)], 0, { live: false, run: run({ parentCall: "call-9", state: "completed" }) })]);
    const onShowCall = vi.fn();
    const w = draw(port, { onShowCall });
    await flushPromises();
    const back = w.get('[data-action="agent.show-call"]');
    expect(back.text()).toBe(t("subagent.tab.back"));
    await back.trigger("click");
    expect(onShowCall).toHaveBeenCalledWith("call-9");
  });
});