<script lang="ts">
import { computed, defineComponent, h, ref, toValue, watchEffect, type MaybeRefOrGetter, type PropType, type Ref, type VNodeChild } from "vue";
import type { ChipCall, Completion, CompletionItem } from "../port/port";
import { chipText, place, reconcile, snap, split, touches, type SkillChip } from "./skillchips";

interface Held {
  // The text these chips are positioned in. Text that moves away from it is an
  // edit the chips have not been carried across yet.
  base: string;
  list: SkillChip[];
}

// useSkillChips keeps the chips in step with a plain textarea. Every change to
// the text, typed or set by code, is reconciled from the text the chips were
// last placed in, so no path into the box has to remember to tell them.
export function useSkillChips(box: Ref<HTMLTextAreaElement | null>, text: MaybeRefOrGetter<string>, set: (text: string, caret: number) => void) {
  const held = ref<Held>({ base: "", list: [] });
  watchEffect(() => {
    const t = toValue(text);
    if (held.value.base === t) return;
    const el = box.value;
    const r = reconcile(held.value.base, t, held.value.list, el ? el.selectionStart : undefined);
    held.value = { base: r.text, list: r.chips };
    if (r.text !== t) set(r.text, r.caret ?? r.text.length);
  }, { flush: "post" });
  const list = computed(() => (held.value.base === toValue(text) ? held.value.list : []));
  return {
    list,
    held: () => held.value,
    restore: (h: Held) => (held.value = h),
    place: (at: string, c: Completion, item: CompletionItem) => {
      const r = c.kind === "slash" ? place(at, c.from, c.to, list.value, item) : null;
      if (r) held.value = { base: r.text, list: r.chips };
      return r;
    },
    touches: (caret: number) => touches(list.value, caret),
    snap: (el: HTMLTextAreaElement, before: number) => {
      if (el.selectionStart !== el.selectionEnd) return el.selectionStart;
      const at = snap(list.value, el.selectionStart, before);
      if (at !== el.selectionStart) el.setSelectionRange(at, at);
      return at;
    },
    call: (compose: (v: string) => string): ChipCall | undefined => {
      if (list.value.length === 0) return undefined;
      const s = split(toValue(text), list.value);
      return { submit: compose(s.submit), invocations: s.invocations };
    },
  };
}

const COPIED = [
  "font-family", "font-size", "font-weight", "font-style", "font-feature-settings", "font-variation-settings",
  "letter-spacing", "word-spacing", "line-height", "text-indent", "text-transform", "tab-size",
  "padding-top", "padding-right", "padding-bottom", "padding-left", "direction",
];

// The chips drawn over the textarea's own glyphs, at the same metrics, so the
// caret and selection stay the textarea's while the words take the chip's look.
// Scrolling follows the textarea's scroll timeline in CSS; --travel is the
// distance that timeline spans.
export const ChipMirror = defineComponent({
  name: "ChipMirror",
  props: {
    box: { type: Object as PropType<Ref<HTMLTextAreaElement | null>>, required: true },
    text: { type: String, required: true },
    chips: { type: Array as PropType<SkillChip[]>, required: true },
  },
  setup(props) {
    const mirror = ref<HTMLDivElement | null>(null);
    const sync = () => {
      const el = props.box.value;
      const m = mirror.value;
      if (!el || !m) return;
      const cs = getComputedStyle(el);
      for (const k of COPIED) m.style.setProperty(k, cs.getPropertyValue(k));
      m.style.paddingRight = `${parseFloat(cs.paddingRight) + el.offsetWidth - el.clientWidth}px`;
      m.style.setProperty("--travel", `${el.scrollHeight - el.clientHeight}px`);
    };
    watchEffect((_onCleanup) => {
      void props.chips;
      void props.text;
      sync();
      const el = props.box.value;
      if (!el || props.chips.length === 0 || typeof ResizeObserver === "undefined") return;
      const ro = new ResizeObserver(sync);
      ro.observe(el);
      _onCleanup(() => ro.disconnect());
    }, { flush: "post" });
    return () => {
      if (props.chips.length === 0) return null;
      const parts: VNodeChild[] = [];
      let from = 0;
      for (const c of props.chips) {
        parts.push(props.text.slice(from, c.at));
        parts.push(h("span", { key: c.at, class: "skillchip", "data-kind": c.kind }, chipText(c)));
        from = c.at + chipText(c).length;
      }
      parts.push(props.text.slice(from) + "\u200b");
      return h("div", { ref: mirror, class: "chipmirror", "aria-hidden": "true" }, [h("div", { class: "chipflow" }, parts)]);
    };
  },
});

export default ChipMirror;
</script>