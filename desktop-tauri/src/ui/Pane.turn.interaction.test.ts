// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/vue";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import "./testkit";
import Pane from "./Pane.vue";
import { MockPort } from "../port/mock";
import type { RuntimeView } from "../port/hub";
import type { PaneReport } from "./panetypes";
import { boot, STORAGE } from "../i18n";

afterEach(() => {
  cleanup();
  localStorage.setItem(STORAGE, "zh");
  boot();
});

const runtime: RuntimeView = {
  id: "r1",
  base: "",
  root: "C:\\work\\demo",
  name: "demo",
  sessionPath: "/sessions/demo.jsonl",
};

function mountPane(port: MockPort) {
  const reports: PaneReport[] = [];
  const view = render(Pane, {
    props: {
      port,
      rt: runtime,
      title: "并行会话演示",
      active: true,
      sideHost: null,
      side: false,
      onFocus: () => {},
      onReport: (_id: string, report: PaneReport) => { reports.push(report); },
      visible: true,
      onSessionChanged: () => {},
      pulse: 0,
      findPulse: 0,
      needsProject: false,
      onOpenProject: () => {},
      onKeepHere: () => {},
      onSettings: () => {},
      theme: "dark",
      dockW: 0,
      dockMax: 0,
      onDockW: () => {},
    },
  });
  return { view, reports };
}

describe("a pane's turn", () => {
  it("renders the streamed answer, stops at the approval and the question, and finishes", async () => {
    const port = new MockPort();
    const { reports } = mountPane(port);
    const user = userEvent.setup();
    const box = screen.getByRole("combobox", { name: /任务输入/ });
    await user.type(box, "把项目里的测试跑一遍，并告诉我结果");
    await user.keyboard("{Enter}");

    expect(await screen.findByText(/先翻历史/, {}, { timeout: 60_000 })).toBeTruthy();
    await waitFor(() => expect(document.querySelector('[data-action="decision.tool"][data-value="once"]')).not.toBeNull(), { timeout: 60_000 });
    fireEvent.click(document.querySelector('[data-action="decision.tool"][data-value="once"]')!);

    await waitFor(() => expect(document.querySelector(".ask-pane .opt")).not.toBeNull(), { timeout: 60_000 });
    const chosen = document.querySelector('[data-action="ask.answer"][data-value="chosen"]') as HTMLButtonElement;
    expect(chosen.disabled).toBe(true);
    fireEvent.click(document.querySelector(".ask-pane .opt")!);
    await waitFor(() => expect(chosen.disabled).toBe(false), { timeout: 20_000 });
    fireEvent.click(chosen);

    expect(await screen.findByText(/两处都改了/, {}, { timeout: 60_000 })).toBeTruthy();
    expect(reports.length).toBeGreaterThan(0);
  }, 120_000);
});
