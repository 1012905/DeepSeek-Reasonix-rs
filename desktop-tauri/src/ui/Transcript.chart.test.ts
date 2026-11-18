// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ref } from "vue";
import { cleanup, render } from "@testing-library/vue";
import "./testkit";
import { boot } from "../i18n";
import type { Item } from "../state/session";
import Transcript from "./Transcript.vue";
import { SALES, chartCall } from "./chart/fixtures";

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

const chartItem: Item = { t: "tool", id: "i1", tool: chartCall(SALES), running: false, children: [] };
const plainTool: Item = { t: "tool", id: "i2", tool: { id: "c2", name: "read_file", args: '{"path":"a.ts"}', output: "contents", readOnly: true }, running: false, children: [] };
const runningChart: Item = { t: "tool", id: "i3", tool: chartCall(SALES, { output: undefined }), running: true, children: [] };

describe("the transcript's chart branch", () => {
  it("draws a settled render_chart call as a chart card, not folded into activity", () => {
    const { container } = draw([chartItem]);
    expect(container.querySelector('.call[data-k="chart"][data-call="call-1"]')).not.toBeNull();
    expect(container.querySelector("figure.chart")).not.toBeNull();
    expect(container.querySelector(".activity-group")).toBeNull();
  });

  it("still folds an ordinary tool call into the activity group", () => {
    const { container } = draw([plainTool]);
    expect(container.querySelector(".activity-group")).not.toBeNull();
    expect(container.querySelector('[data-k="chart"]')).toBeNull();
  });

  it("draws a chart call that has not settled as its tool card", () => {
    const { container } = draw([runningChart]);
    expect(container.querySelector('[data-k="chart"]')).toBeNull();
    expect(container.querySelector(".activity-group")).not.toBeNull();
  });
});