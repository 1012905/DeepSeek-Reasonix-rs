// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/vue";
import "../testkit";
import { boot, t } from "../../i18n";
import ChartCard from "./ChartCard.vue";
import { SALES, withSpec } from "./fixtures";

beforeEach(() => boot());
afterEach(cleanup);

const twoSeries = withSpec({
  data: {
    columns: [{ name: "month", type: "string" }, { name: "revenue", type: "number" }, { name: "cost", type: "number" }],
    rows: [["Jan", 10, 4], ["Feb", 30, 12]],
  },
  marks: [{ type: "bar", x: "month", y: ["revenue", "cost"] }],
});

describe("the chart card", () => {
  it("draws a bar mark under the call's own title, with a readout and a data toggle", async () => {
    const { container } = render(ChartCard, { props: { spec: SALES, callId: "call-1" } });
    const call = container.querySelector('.call[data-k="chart"]');
    expect(call).not.toBeNull();
    expect(call?.getAttribute("data-call")).toBe("call-1");
    expect(container.querySelector("figure.chart")?.getAttribute("aria-label")).toContain("Sales");
    expect(container.querySelector("svg.chart-svg")).not.toBeNull();
    expect(container.querySelector(".chart-read")?.textContent).toBe(t("chat.hover_or_focus_the_chart"));
    expect(container.querySelector(".chart-table")).toBeNull();
    await fireEvent.click(container.querySelector('[data-action="chart.data"]')!);
    expect(container.querySelector(".chart-table")).not.toBeNull();
    expect(container.querySelectorAll(".chart-table tbody tr")).toHaveLength(3);
  });

  it("draws a pie mark in its own pane", () => {
    const pie = withSpec({ marks: [{ type: "pie", x: "month", y: ["revenue"] }] });
    const { container } = render(ChartCard, { props: { spec: pie } });
    expect(container.querySelector(".chart-pie")).not.toBeNull();
  });

  it("names the series in a key only when there is more than one", () => {
    const single = render(ChartCard, { props: { spec: SALES } });
    expect(single.container.querySelector(".chart-mark .chart-key")).toBeNull();
    single.unmount();
    const multi = render(ChartCard, { props: { spec: twoSeries } });
    expect(multi.container.querySelectorAll(".chart-mark .chart-key li")).toHaveLength(2);
  });
});