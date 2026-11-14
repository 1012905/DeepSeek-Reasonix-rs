// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import "./testkit";
import { Appearance } from "./Appearance";
import { usePaint } from "./paint";
import { nextScheduledBoundary, scheduledScheme } from "./scheduledTheme";
import { setThemeSchedule, themeSchedule } from "../state/themeSchedule";
import { MockHub } from "../port/mock_hub";
import { boot, STORAGE, t } from "../i18n";

const overnight = { start: "20:00", end: "07:00" };
const at = (hour: number, minute = 0, second = 0) => new Date(2026, 9, 29, hour, minute, second);
const fail = vi.fn();

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem(STORAGE, "zh");
  boot();
  setThemeSchedule(overnight);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllEnvs(); });

it.each([
  [overnight, 19, 59, "light"], [overnight, 20, 0, "dark"],
  [overnight, 0, 0, "dark"], [overnight, 6, 59, "dark"], [overnight, 7, 0, "light"],
  [{ start: "09:00", end: "17:00" }, 9, 0, "dark"],
  [{ start: "09:00", end: "17:00" }, 17, 0, "light"],
  [{ start: "07:00", end: "07:00" }, 7, 0, "light"],
] as const)("resolves a local half-open daily range %o at %i:%i", (range, hour, minute, scheme) => {
  expect(scheduledScheme(range, at(hour, minute))).toBe(scheme);
});

it("waits only until a real transition, including overnight and sub-minute boundaries", () => {
  expect(nextScheduledBoundary(overnight, at(19, 59, 59))).toBe(1000);
  expect(nextScheduledBoundary(overnight, at(23))).toBe(8 * 60 * 60 * 1000);
  expect(nextScheduledBoundary({ start: "07:00", end: "07:00" }, at(12))).toBeNull();
});

it("persists a valid range, ignores incomplete edits, and defaults corrupt storage", () => {
  setThemeSchedule({ start: "18:30", end: "06:45" });
  expect(themeSchedule()).toEqual({ start: "18:30", end: "06:45" });
  expect(JSON.parse(localStorage.getItem("rx-theme-schedule")!)).toEqual(themeSchedule());
  setThemeSchedule({ start: "", end: "24:00" });
  expect(themeSchedule()).toEqual({ start: "18:30", end: "06:45" });
  localStorage.setItem("rx-theme-schedule", '{"start":"99:00","end":"07:00"}');
  expect(themeSchedule()).toEqual(overnight);
  localStorage.setItem("rx-theme-schedule", "not JSON");
  expect(themeSchedule()).toEqual(overnight);
});

it("keeps a choice for this window when persistence is unavailable", () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new DOMException("unavailable"); });
  setThemeSchedule({ start: "21:00", end: "08:00" });
  expect(themeSchedule()).toEqual({ start: "21:00", end: "08:00" });
});

async function setup() {
  const hub = new MockHub();
  const runtimes = await hub.runtimes();
  const port = hub.machinePort();
  vi.spyOn(port, "themes").mockResolvedValue([{ id: "test", name: "Test", active: true,
    tokens: { light: { bg: "#eeeeee", fg: "#111111" }, dark: { bg: "#111111", fg: "#eeeeee" } } }]);
  const hook = renderHook(() => usePaint(hub, runtimes, false, fail));
  await act(async () => {});
  return { hub, port, ...hook };
}
const packed = () => ((document.getElementById("pack-theme") as HTMLStyleElement)?.sheet?.cssRules[0] as CSSStyleRule)?.style;

it("repaints active packs and contrast exactly at both boundaries, and restores mode/range after remount", async () => {
  localStorage.setItem("rx-theme", "scheduled");
  localStorage.setItem("rx-contrast", "strong");
  vi.useFakeTimers(); vi.setSystemTime(at(19, 59, 59));
  const first = await setup();
  expect(first.result.current.scheme).toBe("light");
  expect(packed().getPropertyValue("--page")).toBe("#eeeeee");
  expect(localStorage.getItem("rx-theme-resolved")).toBe("light");
  act(() => vi.advanceTimersByTime(1000));
  expect(first.result.current.scheme).toBe("dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(packed().getPropertyValue("--page")).toBe("#111111");
  expect(packed().getPropertyValue("--text")).toContain("+ 0.101");
  expect(localStorage.getItem("rx-theme-resolved")).toBe("dark");
  act(() => vi.advanceTimersByTime(11 * 60 * 60 * 1000));
  expect(first.result.current.scheme).toBe("light");
  first.unmount();
  const second = await setup();
  expect(second.result.current.theme).toBe("scheduled");
  expect(second.result.current.scheme).toBe("light");
  expect(themeSchedule()).toEqual(overnight);
});

it("reconciles sleep, visibility and clock changes on focus without replaying missed boundaries", async () => {
  localStorage.setItem("rx-theme", "scheduled");
  vi.useFakeTimers(); vi.setSystemTime(at(19));
  // Watch the hook's own boundary timers (identified by their delay) instead of
  // `getTimerCount()`: since jsdom 31 React's act wrapper leaves an internal
  // guard timer pending on the fake clock, so the global count is no longer
  // the component's.
  const registered = vi.spyOn(window, "setTimeout");
  const cleared = vi.spyOn(window, "clearTimeout");
  const paintWrites = vi.spyOn(Storage.prototype, "setItem");
  const writes = () => paintWrites.mock.calls.filter(([key]) => key === "rx-theme-resolved").length;
  const { result } = await setup();
  // Boundary timers still pending with the given delay: armed, and not yet cleared.
  const liveBoundaries = (delay: number) => {
    const clearedHandles = new Set(cleared.mock.calls.map(([handle]) => String(handle)));
    return registered.mock.calls.filter(([, ms], i) =>
      ms === delay && !clearedHandles.has(String(registered.mock.results[i]?.value))).length;
  };
  expect(liveBoundaries(60 * 60 * 1000)).toBe(1); // 19:00 sits 1h before the 20:00 boundary
  vi.setSystemTime(at(23));
  act(() => window.dispatchEvent(new Event("focus")));
  expect(result.current.scheme).toBe("dark");
  vi.setSystemTime(at(8));
  act(() => document.dispatchEvent(new Event("visibilitychange")));
  expect(result.current.scheme).toBe("light");
  expect(liveBoundaries(60 * 60 * 1000)).toBe(0); // the pre-boundary timer was re-armed
  expect(liveBoundaries(12 * 60 * 60 * 1000)).toBe(1); // exactly once: 08:00 sits 12h before 20:00
  // Stale boundaries were cleared: advancing past them must fire no paint.
  const before = writes();
  act(() => vi.advanceTimersByTime(12 * 60 * 60 * 1000 - 1));
  expect(writes()).toBe(before);
});

it("keeps explicit and system modes authoritative and cancels the schedule when leaving it", async () => {
  vi.useFakeTimers(); vi.setSystemTime(at(23));
  // See "reconciles" above: watch the hook's own timers, not the fake clock's.
  const registered = vi.spyOn(window, "setTimeout");
  const { result } = await setup();
  act(() => result.current.setTheme("light"));
  act(() => setThemeSchedule({ start: "00:00", end: "23:59" }));
  expect(result.current.scheme).toBe("light");
  act(() => result.current.setTheme("dark"));
  expect(result.current.scheme).toBe("dark");
  act(() => result.current.setTheme("auto"));
  expect(result.current.scheme).toBe(matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  act(() => result.current.setTheme("scheduled"));
  expect(result.current.scheme).toBe("dark");
  // Armed exactly once on entering the schedule: the range was rewritten to
  // 00:00–23:59 above, so 23:00 sits 59 minutes before the 23:59 boundary.
  expect(registered.mock.calls.length).toBe(1);
  expect(registered.mock.calls[0]?.[1]).toBe(59 * 60 * 1000);
  act(() => result.current.setTheme("light"));
  // Leaving the schedule re-arms nothing.
  expect(registered.mock.calls.length).toBe(1);
  act(() => vi.advanceTimersByTime(24 * 60 * 60 * 1000));
  expect(result.current.scheme).toBe("light"); // a leaked boundary would repaint dark at 23:00
});

it("changes the real paint through appearance controls and leaves contrast, palettes and fixed modes available", async () => {
  vi.useFakeTimers(); vi.setSystemTime(at(21));
  const { port, result } = await setup();
  function Picker() {
    const p = result.current;
    return <Appearance port={port} theme={p.theme} onTheme={p.setTheme} contrast={p.contrast} onContrast={p.setContrast}
      weight={p.weight} onWeight={p.setWeight} look={p.look} onLook={p.onLook} reloadThemes={p.reloadThemes} />;
  }
  const view = render(<Picker />);
  expect(screen.queryByLabelText(t("深色开始时间"))).toBeNull();
  fireEvent.click(screen.getByRole("radio", { name: t("定时") }));
  view.rerender(<Picker />);
  expect(result.current.scheme).toBe("dark");
  fireEvent.change(screen.getByLabelText(t("深色开始时间")), { target: { value: "22:00" } });
  expect(result.current.scheme).toBe("light");
  expect(screen.getByLabelText(t("恢复浅色时间"))).toBeTruthy();
  expect(screen.getByRole("group", { name: t("look.interface_theme") })).toBeTruthy();
  fireEvent.click(screen.getByRole("radio", { name: t("更强") }));
  view.rerender(<Picker />);
  expect(result.current.contrast).toBe("strong");
  fireEvent.click(screen.getByRole("radio", { name: t("深色") }));
  view.rerender(<Picker />);
  expect(result.current.scheme).toBe("dark");
  expect(screen.queryByLabelText(t("深色开始时间"))).toBeNull();
  expect(themeSchedule().start).toBe("22:00");
});
