import { computed, onWatcherCleanup, ref, toValue, watch, type ComputedRef, type MaybeRefOrGetter } from "vue";

const WINDOW_MS = 2500;
let until = 0;

// Set when first-run setup hands over to the conversation: for a moment after,
// the composer rises into place and types its prompt, once.
export const startRise = () => { until = Date.now() + WINDOW_MS; };
export const isRising = () => Date.now() < until;

const calm = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

// The text typed out over 1.2 seconds right after setup, and whole at any other
// time. Typing into the box, or reduced motion, shows it whole at once.
export function useTypedPlaceholder(full: MaybeRefOrGetter<string>, typing: MaybeRefOrGetter<boolean>): ComputedRef<string> {
  const active = isRising() && !calm();
  const shown = ref(active ? 0 : toValue(full).length);
  watch([() => toValue(full), () => toValue(typing)], ([text, busy]) => {
    if (!active || busy) return;
    const step = Math.max(20, 1200 / Math.max(1, text.length));
    const timer = window.setInterval(() => { shown.value = shown.value >= text.length ? shown.value : shown.value + 1; }, step);
    onWatcherCleanup(() => window.clearInterval(timer));
  }, { immediate: true });
  return computed(() => {
    const text = toValue(full);
    return !active || toValue(typing) || shown.value >= text.length ? text : text.slice(0, shown.value);
  });
}