import { defineComponent, h, type PropType, type VNodeChild } from "vue";

/** A named group of gauges. Nine equal blocks read as one unfilled form: every
 *  label the same size, no answer to "which of these belong together". A group
 *  states the question its rows answer, which is what lets a glance skip six
 *  of them. */
// id names the block the way the panels that build their own frame already do,
// so what the rail put where can be read off the document rather than guessed
// from the words inside it.
export const Grp = defineComponent({
  name: "Grp",
  props: {
    id: { type: String, required: true },
    name: { type: String, required: true },
    aside: { type: null as unknown as PropType<VNodeChild>, default: undefined },
  },
  setup(props, { slots }) {
    return () => h("section", { class: "dgrp", "data-b": props.id }, [
      h("h3", { class: "dgrp-h" }, [
        props.name,
        props.aside != null ? h("span", { class: "a" }, () => props.aside) : null,
      ]),
      slots.default?.(),
    ]);
  },
});

export const Row = defineComponent({
  name: "Row",
  props: {
    k: { type: null as unknown as PropType<VNodeChild>, required: true },
    v: { type: null as unknown as PropType<VNodeChild>, required: true },
    tone: { type: String, default: undefined },
  },
  setup(props) {
    return () => h("div", { class: "drow", "data-tone": props.tone }, [
      h("span", { class: "k" }, () => props.k),
      h("span", { class: "v" }, () => props.v),
    ]);
  },
});

/** Drawn only where a real denominator exists. A bar without one invents a
 *  ceiling out of whatever the largest value happened to be, and the reader
 *  believes it — so token totals get a number and no bar. */
export const Bar = defineComponent({
  name: "Bar",
  props: {
    pct: { type: Number, required: true },
    tone: { type: String, default: undefined },
  },
  setup(props) {
    return () => h("div", { class: "gbar", "data-tone": props.tone }, [
      h("i", { style: { width: `${Math.max(0, Math.min(100, props.pct))}%` } }),
    ]);
  },
});
