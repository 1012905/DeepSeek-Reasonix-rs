// @vitest-environment jsdom
import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import CopyButton from "./CopyButton.vue";
import TooltipButton from "./TooltipButton.vue";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
describe("clipboard control", () => {
  it("reports success and failure without losing the button label", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const wrapper = mount(CopyButton, { props: { text: "answer", label: "Copy" } });
    await wrapper.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("answer");
    expect(wrapper.attributes("data-state")).toBe("done");
    writeText.mockRejectedValueOnce(new Error("denied"));
    await wrapper.trigger("click");
    await flushPromises();
    expect(wrapper.attributes("data-state")).toBe("failed");
    expect(wrapper.attributes("aria-label")).toBe("Copy");
    wrapper.unmount();
  });
  it("closes its focus tooltip with Escape and removes it on unmount", async () => {
    const wrapper = mount(TooltipButton, { props: { title: "Copy" }, attachTo: document.body });
    await wrapper.get("button").trigger("focus");
    expect(document.querySelector('[role="tooltip"]')?.textContent).toContain("Copy");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    wrapper.unmount();
  });
});
