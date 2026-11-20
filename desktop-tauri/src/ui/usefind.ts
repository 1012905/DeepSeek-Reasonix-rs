import { computed, ref, toValue, watch, type ComputedRef, type MaybeRefOrGetter } from "vue";
import { MAX, search, stepped, type FindMode, type FindScope, type Hit } from "../state/find";
import type { Item } from "../state/session";

const NO_HITS: Hit[] = [];

export interface Finding {
  open: ComputedRef<boolean>;
  query: ComputedRef<string>;
  index: ComputedRef<number>;
  focus: ComputedRef<number>;
  total: ComputedRef<number>;
  capped: ComputedRef<boolean>;
  // n moves on every step, so landing on the same occurrence twice is asked for.
  at: ComputedRef<{ id: string; nth: number; n: number; mode: FindMode } | null>;
  ask: (query: string) => void;
  step: (by: 1 | -1) => void;
  close: () => void;
  /** How the query is read. Off is the plain substring search. */
  regex: ComputedRef<boolean>;
  caseSensitive: ComputedRef<boolean>;
  scope: ComputedRef<FindScope>;
  setMode: (mode: Partial<FindMode>) => void;
}

/** The find bar's whole state. pulse is the window asking; only the pane that
 *  has focus answers it, so one chord does not open a bar in every session. */
export function useFind(items: MaybeRefOrGetter<Item[]>, pulse: MaybeRefOrGetter<number>, active: MaybeRefOrGetter<boolean>, onOpen: () => void, togglePulse: MaybeRefOrGetter<number> = 0): Finding {
  const held = ref<{ q: string; i: number; focus: number; re?: boolean; cs?: boolean; sc?: FindScope } | null>(null);
  const hits = computed(
    () => (held.value ? search(toValue(items), held.value.q, { regex: held.value.re, caseSensitive: held.value.cs, scope: held.value.sc }) : NO_HITS),
  );
  const i = computed(() => (held.value ? Math.min(held.value.i, hits.value.length - 1) : -1));
  const at = computed(() => (i.value >= 0 ? hits.value[i.value] : null));
  // Hits come in reading order, so a row's occurrences sit together.
  const nth = computed(() => {
    let n = 0;
    const here = at.value;
    while (here && i.value - n > 0 && hits.value[i.value - n - 1].id === here.id) n++;
    return n;
  });

  const nonce = ref(0);
  watch([() => at.value?.id, () => at.value?.at], () => {
    if (at.value) nonce.value = nonce.value + 1;
  }, { immediate: true });

  let seen = 0;
  watch([() => toValue(pulse), () => toValue(active)], () => {
    const beat = toValue(pulse);
    if (beat === seen) return;
    seen = beat;
    if (!beat || !toValue(active)) return;
    onOpen();
    held.value = held.value ? { ...held.value, focus: held.value.focus + 1 } : { q: "", i: 0, focus: 1 };
  }, { immediate: true });

  // The title bar button is a switch: the same press that opens the bar closes it.
  let seenToggle = 0;
  watch([() => toValue(togglePulse), () => toValue(active), held], () => {
    const beat = toValue(togglePulse);
    if (beat === seenToggle) return;
    seenToggle = beat;
    if (!beat || !toValue(active)) return;
    if (held.value) {
      held.value = null;
      return;
    }
    onOpen();
    held.value = { q: "", i: 0, focus: 1 };
  }, { immediate: true });

  const total = computed(() => hits.value.length);
  const ask = (q: string) => { held.value = { q, i: 0, focus: held.value?.focus ?? 0 }; };
  const step = (by: 1 | -1) => { if (held.value) held.value = { ...held.value, i: stepped(held.value.i, total.value, by) }; };
  const close = () => { held.value = null; };
  // Changing how the query is read restarts from the first hit: the old index
  // counted hits the new reading may no longer have.
  const setMode = (mode: Partial<FindMode>) => {
    if (!held.value) return;
    held.value = { ...held.value, ...(mode.regex !== undefined ? { re: mode.regex } : {}), ...(mode.caseSensitive !== undefined ? { cs: mode.caseSensitive } : {}), ...(mode.scope !== undefined ? { sc: mode.scope } : {}), i: 0 };
  };

  return {
    open: computed(() => held.value !== null),
    query: computed(() => held.value?.q ?? ""),
    index: computed(() => held.value?.i ?? 0),
    focus: computed(() => held.value?.focus ?? 0),
    total,
    capped: computed(() => total.value >= MAX),
    at: computed(() => (at.value ? { id: at.value.id, nth: nth.value, n: nonce.value, mode: { regex: held.value?.re ?? false, caseSensitive: held.value?.cs ?? false, scope: held.value?.sc ?? "all" } } : null)),
    ask,
    step,
    close,
    regex: computed(() => held.value?.re ?? false),
    caseSensitive: computed(() => held.value?.cs ?? false),
    scope: computed(() => held.value?.sc ?? "all"),
    setMode,
  };
}