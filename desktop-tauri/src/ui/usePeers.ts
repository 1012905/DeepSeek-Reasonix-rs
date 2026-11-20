import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, PeersState } from "../port/port";

const EMPTY: PeersState = { enabled: false, peers: [], held: [] };

export function usePeers(port: MaybeRefOrGetter<AgentPort | null>, everyMs: MaybeRefOrGetter<number>) {
  const state = ref<PeersState>(EMPTY);
  const failed = ref(false);
  let alive = true;
  const refresh = async () => {
    const agent = toValue(port);
    if (!agent) return;
    try {
      const next = await agent.peers();
      if (alive) {
        state.value = next;
        failed.value = false;
      }
    } catch {
      if (alive) failed.value = true;
    }
  };
  watch([() => toValue(port), () => toValue(everyMs)], (_v, _was, onCleanup) => {
    alive = true;
    void refresh();
    const ms = toValue(everyMs);
    const timer = ms > 0
      ? setInterval(() => {
          if (typeof document === "undefined" || !document.hidden) void refresh();
        }, ms)
      : null;
    onCleanup(() => {
      alive = false;
      if (timer) clearInterval(timer);
    });
  }, { immediate: true });
  return { state, failed, refresh };
}