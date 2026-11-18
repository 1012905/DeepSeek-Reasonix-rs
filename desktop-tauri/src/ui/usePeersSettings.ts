import { ref, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import { reason } from "../i18n/kernel";
import type { AgentPort, PeersSettings } from "../port/port";

export interface PeersSettingsStore {
  state: Ref<PeersSettings | null>;
  loadError: Ref<string>;
  publish: (next: PeersSettings) => void;
}

// One read of the settings for everything that shows them: the switches and the
// allowed projects are views of one file, so a save in either one is published
// to both instead of each holding a copy that goes stale.
export function usePeersSettings(port: MaybeRefOrGetter<AgentPort>): PeersSettingsStore {
  const state = ref<PeersSettings | null>(null);
  const loadError = ref("");
  watch(() => toValue(port), (agent, _was, onCleanup) => {
    let alive = true;
    agent.peersSettings().then((s) => { if (alive) state.value = s; }, (e) => { if (alive) loadError.value = reason(e); });
    onCleanup(() => { alive = false; });
  }, { immediate: true });
  const publish = (next: PeersSettings) => { state.value = next; };
  return { state, loadError, publish };
}