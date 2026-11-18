// @vitest-environment jsdom
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserTab } from "../../port/port";
import { MockPort } from "../../port/mock";
import "../testkit";
import BrowserPanel from "./BrowserPanel.vue";
import { resetLogins } from "./BrowserLogin.vue";

// No shell bridge is stubbed here, so this is the plain web build: the column
// cannot lay a native view over anything and has to say which page the agent is
// on instead. That is the branch every browser that did not install the desktop
// shell takes, and the only one a DOM test can reach.
afterEach(() => { resetLogins(); vi.restoreAllMocks(); });

const page = (over: Partial<BrowserTab> = {}): BrowserTab => ({ id: "p1", target: "t1", url: "https://example.com/a", title: "Example", active: true, ...over });

function mountPanel(port: MockPort, extra: Record<string, unknown> = {}) {
  return mount(BrowserPanel, {
    props: {
      port,
      visible: true,
      manual: false,
      onManual: vi.fn(),
      onError: vi.fn(),
      full: false,
      onFull: vi.fn(),
      ...extra,
    },
    attachTo: document.body,
  });
}

describe("the browser column", () => {
  it("names the page the agent is on when this window cannot draw it", async () => {
    const port = new MockPort();
    vi.spyOn(port, "browserTabs").mockResolvedValue([page()]);
    const wrapper = mountPanel(port);
    await flushPromises();
    expect(wrapper.get('[data-page-title]').text()).toBe("Example");
    expect(wrapper.get("[data-page-url]").text()).toBe("https://example.com/a");
    expect(wrapper.get('[role="tab"]').attributes("aria-selected")).toBe("true");
    wrapper.unmount();
  });

  it("starts on the panel's own blank page when the agent has no tabs", async () => {
    const wrapper = mountPanel(new MockPort());
    await flushPromises();
    expect(wrapper.find("[data-page-title]").exists()).toBe(false);
    expect(wrapper.get(".bpage").text()).toContain("内置浏览器");
    wrapper.unmount();
  });

  it("reads the tab list again when the kernel says it moved", async () => {
    const port = new MockPort();
    const read = vi.spyOn(port, "browserTabs").mockResolvedValue([]);
    let push: ((ev: { kind: string }) => void) | undefined;
    vi.spyOn(port, "subscribe").mockImplementation(((onEvent: (ev: { kind: string }) => void) => {
      push = onEvent;
      return () => false;
    }) as unknown as MockPort["subscribe"]);
    const wrapper = mountPanel(port);
    await flushPromises();
    const before = read.mock.calls.length;
    push!({ kind: "browser_tabs_changed" });
    await flushPromises();
    expect(read.mock.calls.length).toBeGreaterThan(before);
    wrapper.unmount();
  });

  it("refuses an address that is not a web page and says why", async () => {
    const port = new MockPort();
    const wrapper = mountPanel(port);
    await flushPromises();
    await wrapper.get("input[data-action='browser.navigate']").setValue("C:\\Users\\me\\page.html");
    await wrapper.get("form.baddr").trigger("submit");
    await flushPromises();
    expect(wrapper.get("[role='alert']").text()).toContain("有效的 http 或 https 地址");
    wrapper.unmount();
  });

  it("opens what was typed through the port, then selects the page it opened", async () => {
    const port = new MockPort();
    const opened = vi.spyOn(port, "browserOpen").mockResolvedValue(page({ id: "p2", target: "t2", url: "https://example.com/b" }));
    vi.spyOn(port, "browserTabs").mockResolvedValue([page({ id: "p2", target: "t2", url: "https://example.com/b" })]);
    const wrapper = mountPanel(port);
    await flushPromises();
    await wrapper.get("input[data-action='browser.navigate']").setValue("example.com/b");
    await wrapper.get("form.baddr").trigger("submit");
    await flushPromises();
    expect(opened).toHaveBeenCalledWith("https://example.com/b", true);
    expect(wrapper.get('[role="tab"]').attributes("data-target")).toBe("p2");
    wrapper.unmount();
  });

  it("closes a tab through the port and goes blank when it was the last one", async () => {
    const port = new MockPort();
    const close = vi.spyOn(port, "browserClose").mockResolvedValue(undefined);
    vi.spyOn(port, "browserTabs").mockResolvedValueOnce([page()]).mockResolvedValue([]);
    const wrapper = mountPanel(port);
    await flushPromises();
    await wrapper.get("[data-action='browser.tab-close']").trigger("click");
    await flushPromises();
    expect(close).toHaveBeenCalledWith("p1");
    expect(wrapper.get(".bpage").text()).toContain("内置浏览器");
    wrapper.unmount();
  });

  it("reports a refused read instead of showing an empty column as the truth", async () => {
    const port = new MockPort();
    vi.spyOn(port, "browserTabs").mockRejectedValue(new Error("the kernel is not there"));
    const onError = vi.fn();
    const wrapper = mountPanel(port, { onError });
    await flushPromises();
    expect(onError).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("asks the pane to leave takeover from the tool strip", async () => {
    const onManual = vi.fn();
    const wrapper = mountPanel(new MockPort(), { manual: true, onManual });
    await flushPromises();
    await wrapper.get("[data-action='browser.manual'][data-value='off']").trigger("click");
    expect(onManual).toHaveBeenCalledWith(false);
    wrapper.unmount();
  });

  it("gives takeover a page of its own to type into", async () => {
    const wrapper = mountPanel(new MockPort(), { manual: true });
    await flushPromises();
    // One book, on the start page, named as such rather than as a bare 浏览器.
    expect(wrapper.findAll('[role="tab"]').length).toBe(1);
    expect(wrapper.get(".studio-browser-start").text()).toContain("内置浏览器");
    wrapper.unmount();
  });

  it("loads a takeover page in a frame of this window's own", async () => {
    const wrapper = mountPanel(new MockPort(), { manual: true });
    await flushPromises();
    await wrapper.get("input[data-action='browser.navigate']").setValue("example.com");
    await wrapper.get("form.baddr").trigger("submit");
    await flushPromises();
    expect(wrapper.get("iframe").attributes("src")).toBe("https://example.com/");
    // The tab is called by the host it reached, so two of them can be told apart.
    expect(wrapper.get('[role="tab"]').text()).toBe("example.com");
    wrapper.unmount();
  });

  it("keeps takeover history behind the back and forward buttons", async () => {
    const wrapper = mountPanel(new MockPort(), { manual: true });
    await flushPromises();
    const back = () => wrapper.get("[data-action='browser.control'][data-value='back']");
    expect(back().attributes("disabled")).toBeDefined();
    await wrapper.get("input[data-action='browser.navigate']").setValue("example.com");
    await wrapper.get("form.baddr").trigger("submit");
    await flushPromises();
    expect(back().attributes("disabled")).toBeUndefined();
    await back().trigger("click");
    await flushPromises();
    expect((wrapper.get("form.baddr input").element as HTMLInputElement).value).toBe("reasonix://start");
    wrapper.unmount();
  });

  it("asks for the workbench's full width and says which side it is on", async () => {
    const onFull = vi.fn();
    const wrapper = mountPanel(new MockPort(), { full: false, onFull });
    await flushPromises();
    const toggle = wrapper.get("[data-action='browser.full']");
    expect(toggle.attributes("aria-pressed")).toBe("false");
    await toggle.trigger("click");
    expect(onFull).toHaveBeenCalledWith(true);
    wrapper.unmount();
  });
});
