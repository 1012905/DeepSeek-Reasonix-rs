// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import "./testkit";
import { boot, t } from "../i18n";
import { decimals } from "../i18n/format";
import type { Waiting } from "../state/session";

const Await = (await import("./Await.vue")).default;

type Retry = NonNullable<Waiting["retry"]>;
const retry = (over: Partial<Retry> = {}): Retry => ({ attempt: 1, max: 3, since: Date.now(), ...over });

beforeEach(() => { boot(); vi.useFakeTimers(); vi.setSystemTime(new Date("2026-01-01T00:00:00Z")); });
afterEach(() => { cleanup(); vi.useRealTimers(); });

const draw = (props: { since: number; retry?: Retry }) => render(Await, { props });

describe("the await notice", () => {
  it("counts the plain wait from the stamp it carries and draws three dots", () => {
    const { container } = draw({ since: Date.now() - 2500 });
    const box = container.querySelector(".await")!;
    expect(box).not.toBeNull();
    expect(box.hasAttribute("data-retry")).toBe(false);
    expect(box.querySelectorAll("i")).toHaveLength(3);
    expect(box.querySelector(".t")!.textContent).toBe(t("composer.waiting_for_a_response_secs", { secs: decimals(2.5, 1) }));
  });

  it("ticks the elapsed clock while it is mounted", async () => {
    const { container } = draw({ since: Date.now() });
    await vi.advanceTimersByTimeAsync(1000);
    expect(container.querySelector(".t")!.textContent).toBe(t("composer.waiting_for_a_response_secs", { secs: decimals(1, 1) }));
  });

  it("names the cause and the backoff before a stream replay", () => {
    const { container } = draw({ since: Date.now(), retry: retry({ attempt: 2, max: 5, scope: "stream", cause: "connection_closed", delayMs: 3000, since: Date.now() - 1000 }) });
    const box = container.querySelector(".await")!;
    expect(box.getAttribute("data-retry")).toBe("");
    const want = `${t("composer.connection_closed")} · ${t("composer.next_replay_attempt_max_in", { attempt: 2, max: 5, n: 2 })}`;
    expect(box.querySelector(".t")!.textContent).toBe(want);
  });

  it("names the timeout limit once the attempt is waiting on headers", () => {
    const { container } = draw({ since: Date.now(), retry: retry({ scope: "headers", cause: "timeout", delayMs: 0, timeoutSecs: 30, since: Date.now() - 1500 }) });
    const want = `${t("composer.the_server_did_not_answer")} · ${t("composer.retry_attempt_max_waiting_for_2", { attempt: 1, max: 3, secs: decimals(1.5, 1), limit: 30 })}`;
    expect(container.querySelector(".t")!.textContent).toBe(want);
  });

  it("reads a zero maximum as the limit the user removed", () => {
    const { container } = draw({ since: Date.now(), retry: retry({ max: 0, scope: "stream", cause: "upstream_error", delayMs: 0, since: Date.now() - 500 }) });
    const want = `${t("composer.the_server_reported_an_error")} · ${t("composer.replay_attempt_max_waiting_for", { attempt: 1, max: "∞", secs: decimals(0.5, 1) })}`;
    expect(container.querySelector(".t")!.textContent).toBe(want);
  });

  it("falls back to the scope when the kernel names no cause", () => {
    const { container } = draw({ since: Date.now(), retry: retry({ scope: "stream", delayMs: 0, since: Date.now() - 200 }) });
    expect(container.querySelector(".t")!.textContent!.startsWith(t("composer.the_response_was_cut_mid"))).toBe(true);
  });
});