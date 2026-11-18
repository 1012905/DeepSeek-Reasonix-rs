// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { computed, defineComponent, provide, type PropType } from "vue";
import { cleanup, fireEvent, render } from "@testing-library/vue";
import "./testkit";
import SayCard from "./cards/SayCard.vue";
import { ThoughtFolds, thoughtKeys, useThoughtFolds } from "./thoughtfold";
import type { Item } from "../state/session";

afterEach(() => { cleanup(); localStorage.clear(); });

type Say = Extract<Item, { t: "say" }>;
// A reload mints every id afresh; only the shape of the conversation survives.
const load = (n: number): Item[] => [
  { t: "user", id: `u${n}`, text: "问", msgIndex: 4 },
  { t: "say", id: `a${n}`, text: "", reasoning: "先想", done: true, thoughtMs: 1000 },
  { t: "say", id: `b${n}`, text: "答", reasoning: "再想", done: true, thoughtMs: 1000 },
];

const Pane = defineComponent({
  components: { SayCard },
  props: {
    items: { type: Array as PropType<Item[]>, required: true },
    scope: { type: String, required: true },
  },
  setup(props) {
    const folds = useThoughtFolds(() => props.items, () => props.scope);
    provide(ThoughtFolds, folds);
    const says = computed(() => props.items.filter((i): i is Say => i.t === "say"));
    return { says };
  },
  template: `<SayCard v-for="i in says" :key="i.id" :item="i" />`,
});

const thoughts = () => [...document.querySelectorAll<HTMLDetailsElement>("details.think")];
const toggle = (d: HTMLDetailsElement) => { d.open = !d.open; fireEvent(d, new Event("toggle")); };

describe("a thought the reader opened", () => {
  it("names each thought by its turn and its place in it", () => {
    expect([...thoughtKeys(load(1)).entries()]).toEqual([["a1", "4.0"], ["b1", "4.1"]]);
  });

  it("is drawn open again after a reload, and only that one", () => {
    const first = render(Pane, { props: { items: load(1), scope: "/s/a.jsonl" } });
    toggle(thoughts()[1]);
    expect(thoughts()[1].open).toBe(true);
    first.unmount();
    render(Pane, { props: { items: load(2), scope: "/s/a.jsonl" } });
    expect(thoughts().map((d) => d.open)).toEqual([false, true]);
  });

  it("keeps a closed choice too, and keeps it to its own conversation", () => {
    const first = render(Pane, { props: { items: load(1), scope: "/s/a.jsonl" } });
    toggle(thoughts()[0]);
    toggle(thoughts()[0]);
    first.unmount();
    render(Pane, { props: { items: load(2), scope: "/s/b.jsonl" } });
    expect(thoughts().map((d) => d.open)).toEqual([false, false]);
    cleanup();
    localStorage.setItem("rx-thought-folds:/s/a.jsonl", JSON.stringify({ "4.0": true }));
    render(Pane, { props: { items: load(3), scope: "/s/a.jsonl" } });
    expect(thoughts()[0].open).toBe(true);
  });

  it("remembers nothing for a conversation not yet saved", () => {
    const first = render(Pane, { props: { items: load(1), scope: "" } });
    toggle(thoughts()[0]);
    first.unmount();
    expect(Object.keys(localStorage).filter((k) => k.startsWith("rx-thought-folds"))).toEqual([]);
  });
});