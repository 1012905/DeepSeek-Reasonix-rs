import { computed, ref, toValue, type MaybeRefOrGetter } from "vue";
import type { Item } from "../state/session";

/** The tool calls a reader asked to see again at the end of the transcript. A
 *  view over calls the transcript already holds, never a fact of the session:
 *  it is not written anywhere, and a call that is no longer in the transcript —
 *  rewound away, or the pane rebuilt — drops out of the list by itself. */
export function useBroughtResults(items: MaybeRefOrGetter<Item[]>) {
  const asked = ref<string[]>([]);
  const here = computed(() => {
    const ids = new Set<string>();
    for (const it of toValue(items)) if (it.t === "tool" && it.tool.id) ids.add(it.tool.id);
    return ids;
  });
  const ids = computed(() => asked.value.filter((id) => here.value.has(id)));
  const bring = (id: string) => { asked.value = [...asked.value.filter((x) => x !== id), id]; };
  const dismiss = (id: string) => { asked.value = asked.value.filter((x) => x !== id); };
  return { ids, bring, dismiss };
}