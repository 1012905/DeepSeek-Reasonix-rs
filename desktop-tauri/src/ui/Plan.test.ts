// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import "./testkit";
import Plan from "./Plan.vue";
import { boot, t } from "../i18n";
import type { PlanStep } from "../state/session";

beforeEach(boot);
afterEach(cleanup);

/** The first unticked row is the one in hand, as a host-driven plan reads. */
const steps = (...done: boolean[]): PlanStep[] =>
  done.map((d, i) => ({
    text: `Step ${i + 1}`,
    status: d ? "completed" : i === done.indexOf(false) ? "in_progress" : "pending",
  }));

const rows = (container: Element) => [...container.querySelectorAll<HTMLElement>(".plan .s")];

describe("the plan block", () => {
  it("counts what is done against what there is, and fills the bar to match", () => {
    const { container } = render(Plan, { props: { steps: steps(true, true, false, false) } });
    expect(container.querySelector('[data-b="plan"]')).not.toBeNull();
    expect(rows(container)).toHaveLength(4);
    expect(container.querySelector(".lbl .c")?.textContent?.trim()).toBe("2 / 4");
    expect(container.querySelector<HTMLElement>(".prog2 .track i")?.style.width).toBe("50%");
  });

  it("marks exactly the row in hand as now and the finished rows as done", () => {
    const { container } = render(Plan, { props: { steps: steps(true, false, false) } });
    const drawn = rows(container);
    expect(drawn.map((r) => r.hasAttribute("data-done"))).toEqual([true, false, false]);
    expect(drawn.map((r) => r.hasAttribute("data-now"))).toEqual([false, true, false]);
    expect(drawn[0].querySelector(".b")?.getAttribute("aria-label")).toBe(t("common.done_2"));
    expect(drawn[1].querySelector(".b")?.getAttribute("aria-label")).toBe(t("plan.step.now"));
    expect(drawn[2].querySelector(".b")?.getAttribute("aria-label")).toBe(t("plan.step.todo"));
  });

  it("points at no row once every step is done or the host has started none", () => {
    const settled = render(Plan, { props: { steps: steps(true, true) } });
    expect(rows(settled.container).some((r) => r.hasAttribute("data-now"))).toBe(false);
    cleanup();
    const written = render(Plan, { props: { steps: [{ text: "One", status: "pending" }, { text: "Two", status: "pending" }] } });
    expect(rows(written.container).some((r) => r.hasAttribute("data-now"))).toBe(false);
  });

  it("marks the plan paused only while the turn waits on a person", async () => {
    const props = { steps: steps(true, false) };
    const { container, rerender } = render(Plan, { props });
    expect(container.querySelector(".plan")?.hasAttribute("data-paused")).toBe(false);
    await rerender({ ...props, paused: true });
    expect(container.querySelector(".plan")?.hasAttribute("data-paused")).toBe(true);
  });

  it("draws nothing when there is no plan", () => {
    const { container } = render(Plan, { props: { steps: [] } });
    expect(container.querySelector('[data-b="plan"]')).toBeNull();
    expect(container.querySelector(".plan")).toBeNull();
    expect(container.textContent).toBe("");
  });

  it("stands down where the task tab already draws the same list", () => {
    const { container } = render(Plan, { props: { steps: steps(true, false), shownElsewhere: true } });
    expect(container.querySelector('[data-b="plan"]')).toBeNull();
    expect(container.textContent).toBe("");
  });
});
