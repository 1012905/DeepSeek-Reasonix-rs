import { computed, onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { useShown } from "./shown";

// The kernel pushes 2-3 characters per frame, but in bursts: measured against a
// live turn, the median gap between frames is 0ms and the 90th is 32ms. Drawing
// each arrival reproduces that jitter as visible stutter. Revealing on the frame
// clock instead turns the same bytes into steady typing.
//
// The backlog drains proportionally rather than at a fixed rate, so a burst is
// absorbed within a few frames and the text never falls meaningfully behind.
const DRAIN = 5;

export function useRevealed(text: MaybeRefOrGetter<string>, streaming: MaybeRefOrGetter<boolean | undefined>) {
  const watched = useShown();
  const shown = ref(toValue(text).length);
  let at = toValue(text).length;
  let want = at;
  let raf = 0;

  const stop = () => {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };

  watch(
    [() => toValue(text), () => toValue(streaming)],
    () => {
      const source = toValue(text);
      const live = !!toValue(streaming);
      stop();
      want = source.length;
      // A settled message, a reload or a new turn has no backlog to pace: whatever
      // is there is already final.
      if (!live || !watched || (typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches)) {
        at = want;
        shown.value = want;
        return;
      }
      // Replaced rather than appended (session switch, history restore): the
      // cursor has to come back with the text, or every later growth below the
      // old length is short-circuited below and the reveal never animates again.
      if (at > want) {
        at = want;
        shown.value = want;
      }
      if (at >= want) return;
      const step = () => {
        if (at >= want) {
          raf = 0;
          return;
        }
        at = Math.min(want, at + Math.max(1, Math.ceil((want - at) / DRAIN)));
        shown.value = at;
        raf = at < want ? requestAnimationFrame(step) : 0;
      };
      raf = requestAnimationFrame(step);
    },
    { immediate: true },
  );

  onScopeDispose(stop);

  return computed(() => (toValue(streaming) && watched ? toValue(text).slice(0, shown.value) : toValue(text)));
}
