// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/vue";
import { flushPromises } from "@vue/test-utils";
import { createVNode, defineComponent, h, isVNode, nextTick } from "vue";
import type { ToolOutputPage } from "../port/session";
import type { StateTool } from "../state/outputcap";
import "./testkit";
import Boundary from "./Boundary.vue";
import ToolOutput from "./cards/ToolOutput.vue";
import { useFullOutput, type ReadOutput } from "./cards/useFullOutput";
import { splitLines } from "./codelines";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const Flaky = defineComponent({
  props: { text: { type: String, required: true } },
  setup(props) {
    return () => {
      if (props.text.includes("bad")) throw new Error("renderer failed");
      return h("p", { class: "ok" }, props.text);
    };
  },
});

const Harness = defineComponent({
  props: { text: { type: String, required: true } },
  setup(props) {
    return () => createVNode(Boundary, { retryKey: props.text, fallback: h("div", { class: "fb" }, props.text) }, { default: () => h(Flaky, { text: props.text }) });
  },
});

const renderOutput = (full: ReturnType<typeof useFullOutput>) => render(defineComponent({
  setup: () => () => createVNode(Boundary, { fallback: h("p", "render failed") }, {
    default: () => [full.note, h(ToolOutput, { name: "shell", text: full.text ?? "" })],
  }),
}));

const clipped: StateTool = { id: "call", name: "shell", readOnly: true, output: "start\n⋯\nend", hidden: 100 };
const page = (text: string, offset = 0, truncated = false): ToolOutputPage => ({ callId: "call", text, bytes: 6, offset, truncated });

describe("a boundary around rendered model output", () => {
  it("tries again when what it renders changes", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { container, rerender } = render(Harness, { props: { text: "half a bad chunk" } });
    await nextTick();
    expect(container.querySelector(".fb")).not.toBeNull();
    await rerender({ text: "the whole answer" });
    expect(container.querySelector(".ok")?.textContent).toBe("the whole answer");
  });

  it("stays on its fallback while nothing changes", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { container, rerender } = render(Harness, { props: { text: "bad" } });
    await nextTick();
    expect(container.querySelector(".fb")).not.toBeNull();
    await rerender({ text: "bad" });
    expect(container.querySelector(".fb")).not.toBeNull();
  });

  it("renders full output through loading and byte-offset pagination without falling back", async () => {
    let finish!: (value: ToolOutputPage) => void;
    const pending = new Promise<ToolOutputPage>((resolve) => { finish = resolve; });
    const read = vi.fn<ReadOutput>().mockReturnValueOnce(pending).mockResolvedValueOnce(page("end", 3));
    const full = useFullOutput(clipped, read);
    expect(full.text).toBe(clipped.output);
    expect(isVNode(full.note)).toBe(true);
    const { container, unmount } = renderOutput(full);
    expect(container.querySelector(".term")?.textContent).toBe(clipped.output);
    await fireEvent.click(container.querySelector<HTMLButtonElement>('[data-action="tool.show-full"]')!);
    expect(screen.getByRole("status")).not.toBeNull();
    expect(container.querySelector('[data-action="tool.show-full"]')).toBeNull();
    expect(read).toHaveBeenCalledExactlyOnceWith("call", 0, 64 * 1024);
    finish(page("你", 0, true));
    await flushPromises();
    expect(read).toHaveBeenNthCalledWith(2, "call", 3, 64 * 1024);
    expect(container.querySelector(".term")?.textContent).toBe("你end");
    expect(full.note).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    expect(clipped.output).toBe("start\n⋯\nend");
    unmount();
    const replay = useFullOutput(clipped, read);
    expect(replay.text).toBe(clipped.output);
    expect(isVNode(replay.note)).toBe(true);
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("keeps the clipped output after a read failure and lets the reader retry", async () => {
    const read = vi.fn<ReadOutput>().mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(page("whole"));
    const full = useFullOutput(clipped, read);
    expect(full.text).toBe(clipped.output);
    const { container } = renderOutput(full);
    await fireEvent.click(container.querySelector<HTMLButtonElement>('[data-action="tool.show-full"]')!);
    await flushPromises();
    expect(screen.getByRole("alert").textContent).toContain("offline");
    expect(container.querySelector(".term")?.textContent).toBe(clipped.output);
    await fireEvent.click(container.querySelector<HTMLButtonElement>('[data-action="tool.show-full"]')!);
    await flushPromises();
    expect(read).toHaveBeenNthCalledWith(2, "call", 0, 64 * 1024);
    expect(container.querySelector(".term")?.textContent).toBe("whole");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(full.note).toBeNull();
  });

  it("preserves absent output and offers no unavailable full-output action", () => {
    const absent = useFullOutput({ name: "shell", readOnly: true });
    expect(absent.text).toBeUndefined();
    expect(absent.note).toBeNull();
    for (const [tool, read] of [[clipped, undefined], [{ ...clipped, id: undefined }, vi.fn<ReadOutput>()]] as const) {
      const full = useFullOutput(tool, read);
      expect(full.text).toBe(clipped.output);
      expect(isVNode(full.note)).toBe(true);
      const { container, unmount } = renderOutput(full);
      expect(container.querySelector(".bound")).not.toBeNull();
      expect(container.querySelector('[data-action="tool.show-full"]')).toBeNull();
      unmount();
    }
  });

  it.each([undefined, 7, "token", Symbol("token")])("renders highlighted lines with key %s without falling back", async (key) => {
    const Highlighted = defineComponent({
      setup: () => () => h("pre", splitLines(h("span", { key, class: "token" }, ["first\n\n", h("b", "last")]))
        .map((row, i) => h("span", { key: i, class: "line" }, row))),
    });
    const { container } = render(defineComponent({
      setup: () => () => createVNode(Boundary, { fallback: h("p", { class: "fb" }, "raw") }, { default: () => h(Highlighted) }),
    }));
    await nextTick();
    expect(container.querySelector(".fb")).toBeNull();
    expect([...container.querySelectorAll(".line")].map((line) => line.textContent)).toEqual(["first", "", "last"]);
    expect(container.querySelector(".line:last-child .token b")?.textContent).toBe("last");
  });
});