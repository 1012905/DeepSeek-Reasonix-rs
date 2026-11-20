// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { cleanup, render, waitFor } from "@testing-library/vue";
import "./testkit";
import { boot } from "../i18n";
import type { Item } from "../state/session";
import Transcript from "./Transcript.vue";

beforeEach(() => {
  boot();
  Element.prototype.scrollIntoView = vi.fn();
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const tool = (n: number): Item => ({
  t: "tool",
  id: `i${n}`,
  tool: { id: `c${n}`, name: "read_file", args: "{}", output: "x", readOnly: true },
  running: false,
  children: [],
});

const base = (items: Item[], focus: { call?: string; item?: string; n: number } | null, hidden = false) => ({
  items,
  revision: 0,
  waiting: {},
  scroll: ref<HTMLDivElement | null>(null),
  hidden,
  focus: focus as never,
  onApprove: async () => {},
  onFullAccess: async () => {},
  onPlan: async () => {},
  onAnswer: async () => {},
  onForget: () => {},
  onExtInvoke: () => {},
  onExtSubmit: () => {},
  needsProject: false,
  onOpenProject: () => {},
  onKeepHere: () => {},
  checkpoints: new Map(),
  onPrepareRewind: async () => {},
  onPrepareFileRevert: async () => {},
  onCommitFileRevert: async () => {},
  onCommitRewind: async () => {},
  entering: [],
  onEntered: () => {},
});

const scrolled = () => (Element.prototype.scrollIntoView as ReturnType<typeof vi.fn>).mock.instances;

describe("the transcript's focus request", () => {
  it("opens the work fold that holds the call and centers it", async () => {
    const { container } = render(Transcript, { props: base([tool(1)], { call: "c1", n: 1 }) as never });
    await waitFor(() => expect(scrolled()).toHaveLength(1));
    expect(container.querySelector<HTMLDetailsElement>("details.activity-group")!.open).toBe(true);
    expect(scrolled()[0]).toBe(container.querySelector('[data-call="c1"]'));
  });

  it("holds the request while the pane is hidden, then lands when it shows", async () => {
    const props = base([tool(1)], { call: "c1", n: 1 }, true);
    const { rerender } = render(Transcript, { props: props as never });
    await waitFor(() => expect(scrolled()).toHaveLength(0));
    await rerender({ ...props, hidden: false } as never);
    await waitFor(() => expect(scrolled()).toHaveLength(1));
  });

  it("expands the earlier steps when the call sits behind the activity slice", async () => {
    const items = Array.from({ length: 8 }, (_, i) => tool(i + 1));
    const { container } = render(Transcript, { props: base(items, { call: "c1", n: 1 }) as never });
    await waitFor(() => expect(scrolled()).toHaveLength(1));
    expect(container.querySelector('[data-action="activity.earlier"]')!.getAttribute("aria-expanded")).toBe("true");
  });

  it("spends a request once", async () => {
    const props = base([tool(1)], { call: "c1", n: 1 });
    const { rerender } = render(Transcript, { props: props as never });
    await waitFor(() => expect(scrolled()).toHaveLength(1));
    await rerender({ ...props, revision: 2 } as never);
    expect(scrolled()).toHaveLength(1);
  });
});
