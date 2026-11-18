// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { nextTick } from "vue";
import PaneTabs from "./PaneTabs.vue";
import { setShowsSessionTabs } from "../state/prefs";
import type { RuntimeView } from "../port/hub";

beforeEach(() => setShowsSessionTabs(true));
afterEach(() => {
  cleanup();
  setShowsSessionTabs(false);
});

const rt = (id: string, over: Partial<RuntimeView> = {}): RuntimeView => ({
  id,
  base: `/rt/${id}`,
  root: "C:/work",
  name: id,
  sessionPath: `${id}.jsonl`,
  ...over,
});

const props = (runtimes: RuntimeView[], active: string) => ({
  runtimes,
  active,
  onSelect: vi.fn(),
  onClose: vi.fn(),
  onNew: vi.fn(),
  onReorder: vi.fn(),
});

const tabs = (el: Element) => [...el.querySelectorAll<HTMLElement>(".ptab[data-pane]")];

describe("the strip of open sessions", () => {
  it("draws one tab per open pane and marks the one in front", () => {
    const { container } = render(PaneTabs, { props: props([rt("a"), rt("b")], "b") });
    expect(tabs(container)).toHaveLength(2);
    expect(container.querySelector('[data-pane="b"]')?.getAttribute("aria-selected")).toBe("true");
    expect(container.querySelector('[data-pane="a"]')?.getAttribute("aria-selected")).toBe("false");
    // The one in front is the one Tab reaches first, and the only one it reaches.
    expect(container.querySelector('[data-pane="b"]')?.getAttribute("tabindex")).toBe("0");
    expect(container.querySelector('[data-pane="a"]')?.getAttribute("tabindex")).toBe("-1");
  });

  it("hands a click to the window rather than switching by itself", () => {
    const p = props([rt("a"), rt("b")], "a");
    const { container } = render(PaneTabs, { props: p });
    container.querySelector<HTMLElement>('[data-pane="b"]')!.click();
    expect(p.onSelect).toHaveBeenCalledWith("b");
  });

  it("closes the tab whose button was pressed, and does not also select it", () => {
    const p = props([rt("a"), rt("b")], "a");
    const { container } = render(PaneTabs, { props: p });
    container.querySelector<HTMLButtonElement>('[data-pane="b"] .ptab-x')!.click();
    expect(p.onClose).toHaveBeenCalledWith("b");
    expect(p.onSelect).not.toHaveBeenCalled();
  });

  it("offers a way to open one more", () => {
    const p = props([rt("a")], "a");
    const { container } = render(PaneTabs, { props: p });
    container.querySelector<HTMLButtonElement>(".ptab-new")!.click();
    expect(p.onNew).toHaveBeenCalledOnce();
  });

  it("says which tabs have something the reader has not seen", () => {
    const p = { ...props([rt("a"), rt("b")], "a"), unread: ["b"] };
    const { container } = render(PaneTabs, { props: p });
    expect(container.querySelector('[data-pane="b"] .pip')?.hasAttribute("data-unread")).toBe(true);
    expect(container.querySelector('[data-pane="a"] .pip')?.hasAttribute("data-unread")).toBe(false);
    expect(container.querySelector('[data-pane="b"] .pip')?.getAttribute("aria-label")).toBe("未读");
  });

  it("falls back to the conversation's own name when the pane has none", () => {
    const { container } = render(PaneTabs, {
      props: props([rt("a", { name: "", sessionPath: "C:/sessions/fix the login bug.jsonl" })], "a"),
    });
    expect(container.querySelector(".ptab-nm")?.textContent).toBe("fix the login bug");
  });

  it("draws nothing once the reader has turned session tabs off", async () => {
    setShowsSessionTabs(false);
    const { container } = render(PaneTabs, { props: props([rt("a"), rt("b")], "a") });
    expect(container.querySelector(".panetabs")).toBeNull();
    setShowsSessionTabs(true);
    await nextTick();
    expect(container.querySelector(".panetabs")).not.toBeNull();
  });
});

describe("reordering by dragging", () => {
  const drag = (el: Element, type: string, init: { x?: number } = {}) => {
    const event = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
    // jsdom ships no DataTransfer; the component writes to it only when it is
    // there, so the move is driven by the events alone.
    Object.defineProperty(event, "clientX", { value: init.x ?? 0 });
    Object.defineProperty(event, "dataTransfer", { value: { effectAllowed: "", dropEffect: "", setData: () => {} } });
    el.dispatchEvent(event);
  };

  it("moves a tab to the slot it was dropped in, counted after the move", () => {
    const p = props([rt("a"), rt("b"), rt("c")], "a");
    const { container } = render(PaneTabs, { props: p });
    const [first, second] = tabs(container);

    drag(first, "dragstart");
    // Past the second tab's midpoint: the end of that tab, not its start.
    Object.defineProperty(second, "getBoundingClientRect", {
      value: () => ({ left: 0, right: 100, top: 0, bottom: 20, width: 100, height: 20, x: 0, y: 0, toJSON: () => ({}) }),
    });
    drag(second, "dragover", { x: 90 });
    drag(second, "drop");

    expect(p.onReorder).toHaveBeenCalledWith(0, 1);
  });

  it("says nothing when a tab is dropped where it already was", () => {
    const p = props([rt("a"), rt("b")], "a");
    const { container } = render(PaneTabs, { props: p });
    const [first] = tabs(container);
    drag(first, "dragstart");
    drag(first, "drop");
    expect(p.onReorder).not.toHaveBeenCalled();
  });

  it("forgets the drag when it ends without a drop", () => {
    const p = props([rt("a"), rt("b")], "a");
    const { container } = render(PaneTabs, { props: p });
    const [first, second] = tabs(container);
    drag(first, "dragstart");
    drag(first, "dragend");
    drag(second, "drop");
    expect(p.onReorder).not.toHaveBeenCalled();
  });
});
