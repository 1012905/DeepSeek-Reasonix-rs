import { onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, ThemePack } from "../port/port";
import { reason } from "../i18n/kernel";

export function useThemeInventory(port: MaybeRefOrGetter<AgentPort>) {
  const packs = ref<ThemePack[]>([]);
  const unread = ref("");
  let requests = 0;
  let generation = 0;

  const load = () => {
    const agent = toValue(port);
    const mine = generation;
    const request = ++requests;
    agent.themes()
      .then((next) => {
        if (mine === generation && requests === request) { packs.value = next; unread.value = ""; }
      })
      .catch((e) => {
        if (mine === generation && requests === request) unread.value = reason(e);
      });
  };

  watch(() => toValue(port), () => {
    generation++;
    packs.value = [];
    unread.value = "";
    load();
  }, { immediate: true });
  onScopeDispose(() => { generation++; });

  return { packs, unread, load };
}