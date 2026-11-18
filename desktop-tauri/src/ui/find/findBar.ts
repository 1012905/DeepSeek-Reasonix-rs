import { computed, onScopeDispose, ref, shallowRef, toValue, watch, watchEffect, type ComputedRef, type MaybeRefOrGetter, type Ref } from "vue";
import { MAX, finderFor, stepped, type FindMode } from "../../state/find";
import { landingBox } from "../findland";
import { currentRange, rangesIn, type FindAt } from "../findpaint";

/** One occurrence, named exactly the way the painter names it — which row, and
 *  which occurrence inside that row — so a count and a highlight cannot drift. */
export type FindSlot = FindAt;

// A word character in the sense "whole word" means. ASCII on purpose: the
// matcher compiles every query without the `u` flag, where a `\p{L}` class
// degrades to the literal set {p,{,L,}}. A query written in a script with no
// word breaks therefore matches wherever its text does, which is the useful
// reading of "whole word" there.
const WORD = "0-9A-Za-z_";

const escapeRe = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** The query as the matcher reads it. Whole word is a rewrite of the query, not
 *  a second matcher: state/find and findpaint already read a regular
 *  expression, so a word boundary rides that path and neither the count nor the
 *  highlight can disagree with the other. */
export function reading(text: string, wholeWord: boolean): string {
  const body = text.trim();
  if (!wholeWord || !body) return text;
  // Lookarounds rather than a word-boundary assertion: a query may begin or
  // end with a character that is not a word character, and that assertion
  // would then never match it.
  return `(?<![${WORD}])(?:${escapeRe(body)})(?![${WORD}])`;
}

/** Every occurrence the painter would draw under `host`, in reading order.
 *
 *  Counted through findpaint's own walk rather than by matching each row's text
 *  as one string: a row's words live in several nodes (emphasis, code, links)
 *  and a hit can cross them, so a count taken anywhere else reports hits with
 *  no range to point at. */
export function hitsIn(host: HTMLElement, query: string, mode: FindMode = {}): FindSlot[] {
  const finder = finderFor(query, mode);
  if (!finder) return [];
  const out: FindSlot[] = [];
  for (const row of host.querySelectorAll<HTMLElement>("[data-item]")) {
    const drawn = rangesIn(row, finder).length;
    const id = row.dataset.item ?? "";
    for (let nth = 0; nth < drawn; nth++) {
      out.push({ id, nth });
      if (out.length >= MAX) return out;
    }
  }
  return out;
}

export interface FindBar {
  query: Ref<string>;
  caseSensitive: Ref<boolean>;
  wholeWord: Ref<boolean>;
  /** The query as the matcher reads it. */
  text: ComputedRef<string>;
  mode: ComputedRef<FindMode>;
  /** Which hit is being looked at; -1 when there is none. */
  position: ComputedRef<number>;
  total: Ref<number>;
  capped: ComputedRef<boolean>;
  current: ComputedRef<FindSlot | null>;
  /** Moves on every landing, so the same occurrence can be asked for twice. */
  at: ComputedRef<{ id: string; nth: number; n: number; mode: FindMode } | null>;
  ask: (text: string) => void;
  step: (by: 1 | -1) => void;
  setCase: (on: boolean) => void;
  setWholeWord: (on: boolean) => void;
  reset: () => void;
}

/** The find bar's state, held against what the transcript has actually drawn.
 *
 *  The transcript mounts every row the session holds, so the row set changes
 *  when the transcript does — a mutation, not a scroll — which is why watching
 *  the subtree keeps the count honest as answers stream in. */
export function useFindBar(
  host: MaybeRefOrGetter<HTMLElement | null>,
  open: MaybeRefOrGetter<boolean>,
  onError?: (e: unknown) => void,
): FindBar {
  const query = ref("");
  const caseSensitive = ref(false);
  const wholeWord = ref(false);
  const index = ref(0);
  const hits = shallowRef<FindSlot[]>([]);

  const text = computed(() => reading(query.value, wholeWord.value));
  const mode = computed<FindMode>(() => ({ regex: wholeWord.value, caseSensitive: caseSensitive.value }));
  const total = computed(() => hits.value.length);
  const capped = computed(() => total.value >= MAX);
  const position = computed(() => (total.value === 0 ? -1 : Math.min(index.value, total.value - 1)));
  const current = computed(() => (position.value < 0 ? null : hits.value[position.value] ?? null));

  let frame = 0;
  const count = () => {
    frame = 0;
    const el = toValue(host);
    if (!el || !text.value.trim()) {
      hits.value = [];
      return;
    }
    try {
      hits.value = hitsIn(el, text.value, mode.value);
    } catch (e) {
      hits.value = [];
      onError?.(e);
    }
  };
  // Coalesced to a frame: a streaming answer mutates the transcript per delta,
  // and a count is a walk of every row.
  const recount = () => {
    if (frame) return;
    frame = typeof requestAnimationFrame === "function" ? requestAnimationFrame(count) : (count(), 0);
  };

  watch([text, mode, () => toValue(host)], recount);
  watch(hits, () => {
    if (index.value >= hits.value.length) index.value = Math.max(0, hits.value.length - 1);
  });

  watch([() => toValue(host), () => toValue(open)], ([el, on], _old, onCleanup) => {
    if (!el || !on) return;
    recount();
    const rows = new MutationObserver(recount);
    rows.observe(el, { childList: true, subtree: true, characterData: true });
    onCleanup(() => rows.disconnect());
  }, { immediate: true });

  onScopeDispose(() => {
    if (frame) cancelAnimationFrame(frame);
  });

  const nonce = ref(0);
  watch([() => current.value?.id, () => current.value?.nth], () => {
    if (current.value) nonce.value += 1;
  }, { immediate: true });

  const ask = (next: string) => {
    query.value = next;
    index.value = 0;
  };
  const step = (by: 1 | -1) => {
    index.value = stepped(index.value, total.value, by);
  };
  // Changing how the query is read restarts from the first hit: the old index
  // counted hits the new reading may no longer have.
  const reread = () => {
    index.value = 0;
  };
  const setCase = (on: boolean) => {
    caseSensitive.value = on;
    reread();
  };
  const setWholeWord = (on: boolean) => {
    wholeWord.value = on;
    reread();
  };
  const reset = () => {
    query.value = "";
    index.value = 0;
    hits.value = [];
  };

  return {
    query, caseSensitive, wholeWord, text, mode, position, total, capped, current,
    at: computed(() => (current.value ? { ...current.value, n: nonce.value, mode: mode.value } : null)),
    ask, step, setCase, setWholeWord, reset,
  };
}

// The bar floats over the transcript's top edge, so a found row is landed this
// far down instead of flush against the top, or it arrives under the bar.
const BAR_CLEARANCE = 64;

/** Scrolls the occurrence being looked at where it is read, each time it moves.
 *  The row may arrive a frame after the hit that named it — a history page
 *  mounts on an answer — so this asks again for a few frames before giving up. */
export function reveal(host: HTMLElement | null, slot: FindSlot | null, query: string, mode: FindMode = {}, tries = 4): void {
  if (!host?.isConnected || !slot) return;
  const finder = finderFor(query, mode);
  const row = host.querySelector<HTMLElement>(`[data-item="${CSS.escape(slot.id)}"]`);
  if (!row || !finder) {
    if (tries > 0) requestAnimationFrame(() => reveal(host, slot, query, mode, tries - 1));
    return;
  }
  // A hit inside a folded group is one nobody can see.
  landingBox(row);
  const range = currentRange(row, finder, slot.nth);
  if (!range || typeof range.getBoundingClientRect !== "function") return;
  const box = range.getBoundingClientRect();
  const view = host.getBoundingClientRect();
  // jsdom reports every box as zero; scrolling on that would move nothing.
  if (box.height === 0 && box.width === 0 && view.height === 0) return;
  // A long output scrolls inside its own clip, and the view scrolling does not
  // reach an occurrence that clip holds below its fold.
  for (let el = range.startContainer.parentElement; el && el !== host; el = el.parentElement) {
    if (el.scrollHeight <= el.clientHeight + 1) continue;
    const at = range.getBoundingClientRect();
    const clip = el.getBoundingClientRect();
    if (at.top < clip.top || at.bottom > clip.bottom) el.scrollTop += at.top - (clip.top + clip.height / 3);
  }
  const top = view.top + BAR_CLEARANCE;
  const bottom = view.bottom - view.height / 4;
  if (box.top < top || box.bottom > bottom) host.scrollTop += box.top - (top + (bottom - top) / 3);
}

/** Lands on each new hit once, after the paint that named it. */
export function useFindReveal(
  host: MaybeRefOrGetter<HTMLElement | null>,
  at: MaybeRefOrGetter<{ id: string; nth: number; n: number; mode?: FindMode } | null>,
  query: MaybeRefOrGetter<string>,
): void {
  let landed = 0;
  watchEffect(() => {
    const el = toValue(host);
    const slot = toValue(at);
    const q = toValue(query);
    if (!el || !slot || landed === slot.n) return;
    landed = slot.n;
    reveal(el, slot, q, slot.mode);
  }, { flush: "post" });
}
