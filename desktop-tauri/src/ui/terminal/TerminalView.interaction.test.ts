// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MockPort } from "../../port/mock";
import TerminalView from "./TerminalView.vue";
import "../testkit";

// jsdom has no canvas, and xterm wants one before it will draw anything.
beforeEach(() => {
  HTMLCanvasElement.prototype.getContext = (() => ({
    measureText: (text: string) => ({ width: text.length * 8 }),
    fillRect() {}, clearRect() {}, beginPath() {}, save() {}, restore() {}, scale() {},
    fillText() {}, setTransform() {}, translate() {}, drawImage() {},
    createLinearGradient: () => ({ addColorStop() {} }),
  })) as unknown as typeof HTMLCanvasElement.prototype.getContext;
});
afterEach(cleanup);

const settle = (ms = 0) => new Promise((r) => setTimeout(r, ms));

describe("a terminal view of its own", () => {
  it("opens a shell on the port when asked, and lists it as a tab", async () => {
    const port = new MockPort();
    const opened = vi.spyOn(port, "openTerminal");
    const { container } = render(TerminalView, { props: { port, onError: () => {}, visible: true } });
    await settle();
    // Nothing is opened on mount: a terminal costs a process, and the view
    // starts with none.
    expect(opened).not.toHaveBeenCalled();
    const first = container.querySelector('[data-action="workbench.launch-terminal"]');
    expect(first).not.toBeNull();
    await fireEvent.click(first!);
    await settle(20);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(container.querySelectorAll('[data-action-click="terminal.select"]')).toHaveLength(1);
  });

  it("asks before closing a terminal that is still running a command", async () => {
    const port = new MockPort();
    const close = vi.spyOn(port, "closeTerminal");
    const read = vi.spyOn(port, "terminals");
    const { container } = render(TerminalView, { props: { port, onError: () => {}, visible: true } });
    await fireEvent.click(container.querySelector('[data-action="workbench.launch-terminal"]')!);
    await settle(20);
    // A shell that is busy is one whose command is still running; ending it
    // ends the command, so the close asks first.
    read.mockResolvedValue([{ id: "t1", name: "Git Bash", shell: "/usr/bin/bash", dir: "~", cols: 80, rows: 24, running: true, busy: true, created: "2026-11-07T00:00:00Z" }]);
    await settle(2100);
    const shut = container.querySelector('[data-action="terminal.close"]');
    expect(shut).not.toBeNull();
    await fireEvent.click(shut!);
    await settle(10);
    expect(close).not.toHaveBeenCalled();
    expect(document.querySelector('[data-action="terminal.close-confirm"]')).not.toBeNull();
    await fireEvent.click(document.querySelector('[data-action="terminal.close-confirm"]')!);
    await settle(10);
    expect(close).toHaveBeenCalledTimes(1);
  });
});
