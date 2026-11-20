// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref } from "vue";
import { setLiveGraceMinutes } from "../state/prefs";
import { useLiveGrace } from "./useLiveGrace";

beforeEach(() => {
  localStorage.clear();
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const MIN = 60_000;

type Runs = Record<string, { run: string; live: boolean }>;

function setup() {
  const runsRef = ref<Runs>({});
  const runs = ref<Runs>(runsRef.value);
  let hook!: ReturnType<typeof useLiveGrace>;
  render(defineComponent({
    setup() {
      hook = useLiveGrace(runsRef, runs);
      return () => h("div");
    },
  }));
  // A pane report moves the run state after the hook has been told, as App does.
  const report = async (id: string, live: boolean) => {
    hook.noteReport(id, live);
    runsRef.value = { ...runsRef.value, [id]: { run: "", live } };
    runs.value = runsRef.value;
    await nextTick();
  };
  return { hook, report };
}

it("drops a finished conversation at once while the grace is off", async () => {
  const { hook, report } = setup();
  await report("a", true);
  await report("a", false);
  expect(hook.listedIds.value(["a"])).toEqual([]);
});

it("keeps a conversation seen finishing until its grace runs out, then redraws without it", async () => {
  setLiveGraceMinutes(10);
  const { hook, report } = setup();
  await report("a", true);
  expect(hook.listedIds.value(["a"])).toEqual(["a"]);
  await report("a", false);
  const during = hook.listedIds.value;
  expect(during(["a"])).toEqual(["a"]);
  vi.advanceTimersByTime(9 * MIN);
  await nextTick();
  expect(hook.listedIds.value(["a"])).toEqual(["a"]);
  vi.advanceTimersByTime(2 * MIN);
  await nextTick();
  expect(hook.listedIds.value(["a"])).toEqual([]);
  // The expiry replaced the function, which is what re-draws a memoised list.
  expect(hook.listedIds.value).not.toBe(during);
});

it("lists a conversation again, with a fresh grace, when it runs once more", async () => {
  setLiveGraceMinutes(10);
  const { hook, report } = setup();
  await report("a", true);
  await report("a", false);
  vi.advanceTimersByTime(11 * MIN);
  await nextTick();
  expect(hook.listedIds.value(["a"])).toEqual([]);
  await report("a", true);
  expect(hook.listedIds.value(["a"])).toEqual(["a"]);
  await report("a", false);
  expect(hook.listedIds.value(["a"])).toEqual(["a"]);
});

it("never lists a conversation that was idle from the start", async () => {
  setLiveGraceMinutes(60);
  const { hook, report } = setup();
  await report("b", false);
  expect(hook.listedIds.value(["b"])).toEqual([]);
});