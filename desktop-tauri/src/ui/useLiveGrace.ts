import { computed, ref, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import { useExternal } from "../state/external";
import { liveGraceMinutes, onLiveGraceChange } from "../state/prefs";
import { listedAsLive } from "./sessionScope";

type Runs = Record<string, { run: string; live: boolean }>;

// Which conversations the running list holds: the live ones, and those seen to
// finish within the grace this machine chose (off by default). `listedIds`
// changes identity when the grace changes and at each expiry, which is what
// re-draws a memoised list when a row's time is up; `noteReport` is told every
// pane report, before the run state moves, so a live-to-idle edge is seen.
export function useLiveGrace(runsRef: Ref<Runs>, runs: MaybeRefOrGetter<Runs>) {
  const grace = useExternal(onLiveGraceChange, liveGraceMinutes);
  const finishedAt: Record<string, number> = {};
  const expiries = ref(0);
  const listedIds = computed(() => {
    // Read so an expiry hands back a new function, which re-draws a memoised list.
    void expiries.value;
    const minutes = grace.value;
    return (ids: string[]) => ids.filter((id) => listedAsLive(!!toValue(runsRef)[id]?.live, finishedAt[id], Date.now(), minutes));
  });
  const noteReport = (id: string, live: boolean) => {
    if (live) delete finishedAt[id];
    else if (toValue(runsRef)[id]?.live) finishedAt[id] = Date.now();
  };
  watch([grace, () => toValue(runs), expiries], (_v, _was, onCleanup) => {
    if (grace.value <= 0) return;
    const now = Date.now();
    const ends = Object.values(finishedAt).map((at) => at + grace.value * 60_000).filter((end) => end > now);
    if (ends.length === 0) return;
    const timer = setTimeout(() => { expiries.value = expiries.value + 1; }, Math.min(...ends) - now + 50);
    onCleanup(() => clearTimeout(timer));
  }, { immediate: true });
  return { listedIds, noteReport };
}