import { toValue, watch, type MaybeRefOrGetter } from "vue";

/** Runs `fn` when `count` changes after mount. A counter is how a request made
 *  twice stays two requests, and mounting is not one. */
export function useOnBump(count: MaybeRefOrGetter<number>, fn: () => void): void {
  let seen = toValue(count);
  watch(() => toValue(count), (n) => {
    if (seen === n) return;
    seen = n;
    fn();
  }, { immediate: true });
}