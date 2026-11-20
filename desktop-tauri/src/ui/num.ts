import { onWatcherCleanup, ref, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import { WINDOW_MS, tokensPerSecond, type Sample } from "../port/tokens";

// Metric values are drawn as they arrived. Easing one from its old reading to
// its new one puts numbers on screen that no request ever produced, and nothing
// on screen says which of them were real — so what moves here is geometry and
// emphasis, never a figure a reader would take for a measurement.

// One sample a second while the turn runs, capped at `span`. A rate is the one
// figure on the rail whose shape says more than its current value — whether it
// is climbing, stalling or ragged — and a single number cannot hold that. The
// timer stops when the turn does: an idle rail must not tick.
export function useTrail(value: MaybeRefOrGetter<number>, on: MaybeRefOrGetter<boolean>, span = 60): Ref<number[]> {
  const trail = ref<number[]>([]);
  watch(() => toValue(on), (on) => {
    if (!on) return;
    const id = setInterval(() => { trail.value = [...trail.value, toValue(value)].slice(-span); }, 1000);
    onWatcherCleanup(() => clearInterval(id));
  }, { immediate: true });
  return trail;
}

// A rate is an observation of what arrived, so only an arrival may make one.
// Wall time gets one power over it and no other: to end it. Between arrivals
// the last observation stands, and at the window's edge it goes to zero in a
// single step rather than sliding there through readings nothing produced.
export function useRate(win: MaybeRefOrGetter<Sample[]>, on: MaybeRefOrGetter<boolean>): Ref<number> {
  const rate = ref(0);
  watch([() => toValue(win), () => toValue(on)], ([win, on]) => {
    const last = on ? win[win.length - 1] : undefined;
    if (!last) {
      rate.value = 0;
      return;
    }
    rate.value = tokensPerSecond(win, Date.now());
    const id = setTimeout(() => { rate.value = 0; }, Math.max(0, last.t + WINDOW_MS - Date.now()));
    onWatcherCleanup(() => clearTimeout(id));
  }, { immediate: true });
  return rate;
}