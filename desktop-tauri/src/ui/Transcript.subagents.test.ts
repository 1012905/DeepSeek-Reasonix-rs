// @vitest-environment jsdom
//
// What this file proves: parallel delegations read as a group of their own,
// outside the work fold, while a lone delegate and ordinary work stay in the
// activity group — the lane split the React transcript's WorkLanes carries
// (SubagentGroup.tsx:24-48) and the Vue transcript's single activity batch had
// collapsed into one fold.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ref } from "vue";
import { cleanup, render } from "@testing-library/vue";
import "./testkit";
import { boot } from "../i18n";
import type { Item } from "../state/session";
import Transcript from "./Transcript.vue";

beforeEach(() => boot());
afterEach(cleanup);

const props = (items: Item[]) => ({
  items, revision: 0, waiting: {}, scroll: ref(null), hidden: false, focus: null,
  onApprove: async () => {}, onFullAccess: async () => {}, onPlan: async () => {}, onAnswer: async () => {},
  onForget: () => {}, onExtInvoke: () => {}, onExtSubmit: () => {},
  needsProject: false, onOpenProject: () => {}, onKeepHere: () => {},
  checkpoints: new Map(), onPrepareRewind: async () => {}, onPrepareFileRevert: async () => {},
  onCommitFileRevert: async () => {}, onCommitRewind: async () => {}, entering: [], onEntered: () => {},
});

const draw = (items: Item[]) => render(Transcript, { props: props(items) as never });

// The kernel marks a dispatch with a profile; the proxy's tool name cannot say it.
const delegate = (id: string, name: string, running: boolean): Item => ({
  t: "tool", id, running, children: [],
  tool: { id: `call-${id}`, name: "use_capability", args: "{}", output: running ? undefined : "done", readOnly: false, profile: { name, count: 1, model: "deepseek/v4" } },
});
const plain = (id: string): Item => ({
  t: "tool", id, running: false, children: [],
  tool: { id: `call-${id}`, name: "read_file", args: '{"path":"a.ts"}', output: "contents", readOnly: true },
});

describe("the transcript's parallel-delegation lane", () => {
  it("draws two side-by-side delegates as their own open group, not folded into activity", () => {
    const { container } = draw([delegate("d1", "探路", true), delegate("d2", "复核", true)]);
    expect(container.querySelector(".agent-group")).not.toBeNull();
    expect(container.querySelector(".activity-group")).toBeNull();
    expect(container.textContent).toContain("探路");
    expect(container.textContent).toContain("复核");
  });

  it("still groups two settled delegates outside the fold, closed with their count", () => {
    const { container } = draw([delegate("d1", "探路", false), delegate("d2", "复核", false)]);
    const group = container.querySelector(".agent-group");
    expect(group).not.toBeNull();
    expect(group!.hasAttribute("data-settled")).toBe(true);
    expect(container.querySelector(".activity-group")).toBeNull();
  });

  it("keeps a lone delegate inside the activity group", () => {
    const { container } = draw([delegate("d1", "探路", true), plain("p1")]);
    expect(container.querySelector(".agent-group")).toBeNull();
    expect(container.querySelector(".activity-group")).not.toBeNull();
  });

  it("leaves ordinary work in the activity group", () => {
    const { container } = draw([plain("p1"), plain("p2")]);
    expect(container.querySelector(".agent-group")).toBeNull();
    expect(container.querySelector(".activity-group")).not.toBeNull();
  });
});