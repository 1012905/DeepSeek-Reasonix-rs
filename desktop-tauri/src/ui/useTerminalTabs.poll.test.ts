// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h, onUpdated } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MockPort } from "../port/mock";
import type { TerminalInfo } from "../port/terminal";
import { useTerminalTabs } from "./useTerminalTabs";
import "./testkit";

beforeEach(() => vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] }));
afterEach(() => { cleanup(); vi.useRealTimers(); });

const info = (id: string): TerminalInfo => ({ id, name: "Git Bash", shell: "/usr/bin/bash", dir: "~", cols: 80, rows: 24, running: true, busy: false, created: "2026-11-07T00:00:00Z" });
const settle = () => new Promise((r) => setTimeout(r, 0));

// A poll that finds nothing new must not write the list at all: the component
// consuming these tabs is drawn again only when a value it reads really moved.
it("does not render again when a poll finds the same terminals", async () => {
  const port = new MockPort();
  const listing = vi.spyOn(port, "terminals");
  listing.mockResolvedValueOnce([]).mockResolvedValue([info("t1")]);
  let renders = 0;
  let hook!: ReturnType<typeof useTerminalTabs>;
  render(defineComponent({
    setup() {
      hook = useTerminalTabs({ port, shown: true, pick: () => {}, setDismissed: () => {}, onExternal: () => {}, onError: () => {}, request: 0 });
      onUpdated(() => { renders += 1; });
      return () => h("div", JSON.stringify(hook.tabs.value));
    },
  }));
  await settle();
  await vi.advanceTimersByTimeAsync(2000);
  await settle();
  const afterChange = renders;
  expect(listing).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(2000);
  await settle();
  await vi.advanceTimersByTimeAsync(2000);
  await settle();
  expect(listing).toHaveBeenCalledTimes(4);
  expect(renders).toBe(afterChange);
});