import { computed, onUnmounted, ref, toValue, watch, type ComputedRef, type MaybeRefOrGetter } from "vue";

/** Whole seconds left until the host skips a question, ticking once a second;
 *  null when it will not be skipped. The deadline lives in the reducer, so a
 *  card remounted by scrolling counts on from where it was. */
export function useAskCountdown(
  skipAt: MaybeRefOrGetter<number | undefined>,
  live: MaybeRefOrGetter<boolean>,
): ComputedRef<number | null> {
  const now = ref(Date.now());
  let timer: number | undefined;
  watch([() => toValue(skipAt), () => toValue(live)], ([at, lv]) => {
    if (timer !== undefined) { window.clearInterval(timer); timer = undefined; }
    if (!lv || at === undefined) return;
    now.value = Date.now();
    timer = window.setInterval(() => (now.value = Date.now()), 1000);
  }, { immediate: true });
  onUnmounted(() => { if (timer !== undefined) window.clearInterval(timer); });
  return computed(() => {
    const at = toValue(skipAt);
    const lv = toValue(live);
    if (!lv || at === undefined) return null;
    return Math.max(Math.ceil((at - now.value) / 1000), 0);
  });
}

export function clock(seconds: number): string {
  const m = Math.floor(seconds / 60);
  return m > 0 ? `${m}:${String(seconds % 60).padStart(2, "0")}` : String(seconds);
}