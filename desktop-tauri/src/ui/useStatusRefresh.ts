import { toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, SessionStatus } from "../port/port";

const FIRST_MS = 1000;
const CAP_MS = 15_000;

/** One way to ask the kernel for a pane's status that does not give up. A turn
 *  boundary is the moment the snapshot is asked for, and the snapshot is what
 *  closes or vouches for a run, so a read lost to the network leaves the pane
 *  believing what it last heard — for a pane nobody is looking at, nothing else
 *  asks again. A failed read is therefore asked again with growing gaps until it
 *  answers, and stops when the pane goes away. */
export function useStatusRefresh(port: MaybeRefOrGetter<AgentPort>, adopt: (status: SessionStatus) => void, settled: () => void) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let tries = 0;
  let alive = true;
  watch(() => toValue(port), (_agent, _was, onCleanup) => {
    alive = true;
    onCleanup(() => {
      alive = false;
      clearTimeout(timer);
      tries = 0;
    });
  }, { immediate: true });
  const refresh = (): Promise<void> => toValue(port).status().then(
    (status) => {
      clearTimeout(timer);
      tries = 0;
      adopt(status);
      settled();
    },
    () => {
      if (!alive) return;
      clearTimeout(timer);
      const gap = Math.min(CAP_MS, FIRST_MS * 2 ** tries++);
      timer = setTimeout(() => void refresh(), gap);
    },
  );
  return refresh;
}