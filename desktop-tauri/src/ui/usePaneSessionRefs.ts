import { ref, toValue, type MaybeRefOrGetter } from "vue";

/** A past conversation the rail asked the focused pane's composer to hold as a
 *  reference. n is a counter so asking for the same one twice is two requests. */
export interface SessionRefRequest {
  n: number;
  id: string;
  title: string;
}

/** Requests keyed by pane, and the one call that makes one for the focused pane.
 *  A pane that is not open has no composer to hold it, so it gets none. */
export function usePaneSessionRefs(active: MaybeRefOrGetter<string>, open: (pane: string) => boolean) {
  const requests = ref<Record<string, SessionRefRequest>>({});
  const reference = (id: string, title: string) => {
    const at = toValue(active);
    if (!open(at)) return;
    requests.value = { ...requests.value, [at]: { n: (requests.value[at]?.n ?? 0) + 1, id, title } };
  };
  return [requests, reference] as const;
}