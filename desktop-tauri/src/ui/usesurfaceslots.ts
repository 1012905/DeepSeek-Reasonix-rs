import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort } from "../port/port";
import type { SessionState } from "../state/session_types";
import { placement } from "./slots";

type Views = SessionState["views"];

/** Where the user put each extension surface, and the split of the pane's views
 *  into those beside the composer and those in the rail. */
export function useSurfaceSlots(port: MaybeRefOrGetter<AgentPort>, views: MaybeRefOrGetter<Views>, fail: (e: unknown) => void) {
  const slots = ref<Record<string, string>>({});

  watch(() => toValue(port), (agent) => {
    void agent.surfaceSlots().then((next) => { slots.value = next; }).catch(() => { slots.value = {}; });
  }, { immediate: true });

  // Updated in place: a move is the user's own action, so waiting for a round
  // trip would read as a dead click.
  const moveSurface = async (ext: { pluginId: string; surfaceId: string }, slot: string) => {
    const id = `${ext.pluginId}:${ext.surfaceId}`;
    const next = { ...slots.value };
    if (slot) next[id] = slot;
    else delete next[id];
    slots.value = next;
    await toValue(port).assignSurface(id, slot).catch(fail);
  };

  // Split once per change rather than per frame: a new array each render is a
  // changed prop, and that alone would keep the rail re-rendering all turn.
  const split = computed(() => {
    const at: Views = [];
    const rail: Views = [];
    for (const v of toValue(views)) (placement(v, slots.value) === "composer-trailing" ? at : rail).push(v);
    return [at, rail] as const;
  });
  const atComposer = computed(() => split.value[0]);
  const inRail = computed(() => split.value[1]);

  return { slots, moveSurface, atComposer, inRail };
}