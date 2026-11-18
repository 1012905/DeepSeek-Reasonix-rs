// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h } from "vue";
import "./testkit";
import type { AgentPort } from "../port/port";
import { useLinkRouting } from "./links";

const state = vi.hoisted(() => ({
  draws: true,
  openPage: vi.fn(),
  reveal: vi.fn(),
  error: vi.fn(),
}));

vi.mock("../port/host", () => ({ host: () => ({ drawsBrowserViews: () => state.draws }) }));
vi.mock("./browser_open", () => ({ openPage: state.openPage }));

const Harness = defineComponent({
  props: { port: { type: Object, required: true } },
  setup(props) {
    useLinkRouting(() => props.port as AgentPort, () => state.reveal(), (e) => state.error(e));
    return () => h("div", [
      h("a", { href: "https://example.com/a" }, "go"),
      h("a", { href: "mailto:x@y.z" }, "mail"),
    ]);
  },
});

const port = () => ({ openExternal: vi.fn().mockResolvedValue(undefined) }) as unknown as AgentPort;

beforeEach(() => {
  state.draws = true;
  state.openPage.mockReset().mockResolvedValue({ id: "k1" });
  state.reveal.mockClear();
  state.error.mockClear();
});
afterEach(() => cleanup());

const click = (el: Element, init: MouseEventInit = {}) => {
  const ev = new MouseEvent("click", { button: 0, bubbles: true, cancelable: true, ...init });
  el.dispatchEvent(ev);
  return ev;
};

describe("where a link in this window goes", () => {
  it("opens an http link in the browser this window draws and reveals it", () => {
    const p = port();
    const { container } = render(Harness, { props: { port: p } });
    const ev = click(container.querySelector("a")!);
    expect(ev.defaultPrevented).toBe(true);
    expect(state.openPage).toHaveBeenCalledWith(p, "https://example.com/a");
    expect(state.reveal).toHaveBeenCalled();
    expect(p.openExternal).not.toHaveBeenCalled();
  });

  it("sends a modifier click to the machine's own browser", () => {
    const p = port();
    const { container } = render(Harness, { props: { port: p } });
    click(container.querySelector("a")!, { ctrlKey: true });
    expect(p.openExternal).toHaveBeenCalledWith("https://example.com/a");
    expect(state.openPage).not.toHaveBeenCalled();
  });

  it("falls back to the machine's browser when the shell draws no views", () => {
    state.draws = false;
    const p = port();
    const { container } = render(Harness, { props: { port: p } });
    click(container.querySelector("a")!);
    expect(p.openExternal).toHaveBeenCalledWith("https://example.com/a");
    expect(state.openPage).not.toHaveBeenCalled();
  });

  it("leaves a link that is not http alone", () => {
    const p = port();
    const { container } = render(Harness, { props: { port: p } });
    const ev = click(container.querySelectorAll("a")[1]);
    expect(ev.defaultPrevented).toBe(false);
    expect(state.openPage).not.toHaveBeenCalled();
    expect(p.openExternal).not.toHaveBeenCalled();
  });

  it("opens externally when the in-window page fails without a tab of its own", async () => {
    state.openPage.mockRejectedValue(new Error("no view"));
    const p = port();
    const { container } = render(Harness, { props: { port: p } });
    click(container.querySelector("a")!);
    await vi.waitFor(() => expect(p.openExternal).toHaveBeenCalledWith("https://example.com/a"));
  });
});