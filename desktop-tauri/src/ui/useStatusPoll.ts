import { toValue, watch, type MaybeRefOrGetter } from "vue";

const STATUS_POLL_MS = 250;

// The next read is scheduled when the previous one settles, never on a clock: a
// kernel slower than the cadence would otherwise be asked again before it had
// answered, and the queue of unanswered reads is what starves the window's other
// requests of connections.
export function useStatusPoll(on: MaybeRefOrGetter<boolean>, read: () => Promise<unknown>) {
  watch(() => toValue(on), (enabled, _was, onCleanup) => {
    if (!enabled) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = () => {
      const next = () => {
        if (!stopped) timer = setTimeout(tick, STATUS_POLL_MS);
      };
      void read().then(next, next);
    };
    tick();
    onCleanup(() => {
      stopped = true;
      clearTimeout(timer);
    });
  }, { immediate: true });
}