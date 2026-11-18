<script lang="ts">
import { computed, defineComponent, h, onMounted, onScopeDispose, ref, shallowRef, toValue, watch, type ComponentPublicInstance, type ComputedRef, type MaybeRefOrGetter, type PropType, type Ref } from "vue";
import { t } from "../i18n";
import type { AgentPort, Completion, CompletionItem } from "../port/port";
import { chipKind } from "./skillchips";
import StudioIcon from "./StudioIcon.vue";

const EMPTY: Completion = { kind: "", from: 0, to: 0, items: [] };

// What the right-hand column says a row is. A file and a command look nothing
// alike in what they do, and the menu is the only place that difference shows.
const KIND: Record<string, string> = {
  builtin: "common.built_in",
  command: "chat.commands",
  skill: "settings.skills",
  subagent: "common.subagents",
  prompt: "MCP",
  file: "common.files",
  dir: "chat.folder",
  resource: "chat.resource",
  session: "common.session",
};

const HINT: Record<string, string> = {
  ref: "chat.selects_enter_references_tab_completes",
  session: "chat.choose_enter_adds_it_to",
  slash: "chat.selects_tab_or_enter_completes",
  "slash-arg": "chat.selects_tab_or_enter_completes",
};

// Asking the kernel costs one local round trip, so the gate is only about not
// asking at all while ordinary prose is being typed. It is deliberately looser
// than the real grammar — '@' mid-word and '/' mid-line are both rejected on
// the other side — because a false yes costs a request and a false no costs a
// menu that never opens.
function mightComplete(text: string): boolean {
  return text.startsWith("/") || text.includes("@") || text.includes("#");
}

// A "/" opening a word past the start of the line can only become a chip. The
// kernel reads a slash at the start alone, so the word is asked about as if it
// opened the line, and only what a sentence can hold comes back.
function midSlash(text: string, caret: number): { from: number; to: number; token: string } | null {
  const from = text.slice(0, caret).search(/\/\S*$/);
  if (from <= 0 || !/\s/.test(text[from - 1])) return null;
  const rest = text.slice(from).search(/\s/);
  return { from, to: rest < 0 ? text.length : from + rest, token: text.slice(from, caret) };
}

// Whether the caret rests in a word that opens with "/" at the line start or
// after a space: the one place a person is waiting for the command list. Prose
// typed after a finished command still asks (an argument may complete) but is
// not waiting, so it never announces loading.
function inSlashWord(text: string, caret: number): boolean {
  return /(^|\s)\/\S*$/.test(text.slice(0, caret));
}

// The chips a menu pick can become, and whether the caret rests on one: a chip
// is finished, so the letters it is spelled with are not a word to complete.
interface ChipHost {
  place: (text: string, c: Completion, item: CompletionItem) => { text: string; caret: number } | null;
  touches: (caret: number) => boolean;
}

function ask(port: Pick<AgentPort, "complete">, text: string, caret: number, mid: ReturnType<typeof midSlash>, signal: AbortSignal): Promise<Completion> {
  if (!mid) return port.complete(text, caret, signal).then(withTyped);
  return port.complete(mid.token, mid.token.length, signal).then((r) => ({
    kind: "slash",
    from: mid.from,
    to: mid.to,
    query: r.query,
    items: (r.items ?? []).filter((it) => chipKind(it) !== null),
  }));
}

function withTyped(r: Completion): Completion {
  return r.typed && r.items?.length ? { ...r, items: [r.typed, ...r.items] } : r;
}

export interface CompletionState {
  completion: Ref<Completion>;
  active: ComputedRef<number>;
  open: ComputedRef<boolean>;
  // The line already spells the highlighted command in full.
  typedChosen: ComputedRef<boolean>;
  loading: Ref<boolean>;
  // Whether the keys still hold the highlight. A menu opens under whatever the
  // pointer happens to be resting on, so the pointer only takes it by moving.
  kb: Ref<boolean>;
  move: (delta: number) => void;
  hover: (i: number) => void;
  dismiss: () => void;
  accept: (item?: CompletionItem, mode?: "confirm" | "descend") => void;
}

// useCompletion keeps the menu in step with the caret. Every decision about
// what a token means — where it starts, what may replace it, how a path with
// spaces is escaped — comes from the kernel, so this hook only asks, renders,
// and splices the answer back in.
export function useCompletion(
  port: MaybeRefOrGetter<Pick<AgentPort, "complete">>,
  text: MaybeRefOrGetter<string>,
  caret: MaybeRefOrGetter<number>,
  apply: (text: string, caret: number) => void,
  chips?: ChipHost,
  onSession?: (picked: { id: string; title: string }) => void,
): CompletionState {
  const completion = shallowRef<Completion>(EMPTY);
  const active = ref(0);
  const kb = ref(true);
  // An answer and its default highlight publish together; a deferred reset can
  // overwrite navigation after the rows are already visible.
  const setCompletion = (next: Completion) => {
    completion.value = next;
    active.value = 0;
    kb.value = true;
  };
  const loading = ref(false);
  // The answer that was dismissed, not a flag: a reset run as an effect lands
  // after the commit that opened the menu, and a blur between the two is undone.
  const dismissed = shallowRef<Completion | null>(null);
  // Answers can land out of order; only the newest question still has an answer
  // worth showing.
  let asked = 0;
  let asking: AbortController | null = null;

  watch([() => toValue(port), () => toValue(text), () => toValue(caret)], (_value, _old, onCleanup) => {
    const id = ++asked;
    const value = toValue(text);
    const at = toValue(caret);
    const onChip = chips?.touches(at) ?? false;
    const mid = onChip ? null : midSlash(value, at);
    if (onChip || (!mid && !mightComplete(value))) {
      setCompletion(EMPTY);
      loading.value = false;
      return;
    }
    const previous = completion.value;
    const slashy = mid !== null || value.startsWith("/");
    const compatible = slashy
      ? previous.kind === "slash" || previous.kind === "slash-arg"
      : previous.kind === "ref" || previous.kind === "session";
    if (!compatible) setCompletion(EMPTY);
    loading.value = false;
    const timer = inSlashWord(value, at) && !compatible
      ? window.setTimeout(() => { if (id === asked) loading.value = true; }, 180)
      : undefined;
    const controller = new AbortController();
    asking = controller;
    ask(toValue(port), value, at, mid, controller.signal)
      .then((r) => {
        if (id !== asked) return;
        window.clearTimeout(timer);
        setCompletion(r.items?.length ? r : EMPTY);
        loading.value = false;
      })
      .catch(() => {
        if (id !== asked) return;
        window.clearTimeout(timer);
        setCompletion(EMPTY);
        loading.value = false;
      });
    onCleanup(() => {
      window.clearTimeout(timer);
      controller.abort();
    });
  }, { immediate: true });

  onScopeDispose(() => asking?.abort());

  const items = computed(() => completion.value.items);
  const open = computed(() => dismissed.value !== completion.value && items.value.length > 0);
  const shown = computed(() => Math.min(active.value, items.value.length - 1));

  const accept = (item?: CompletionItem, mode: "confirm" | "descend" = "confirm") => {
    const pick = item ?? items.value[Math.min(active.value, items.value.length - 1)];
    if (!pick) return;
    const value = toValue(text);
    const current = completion.value;
    if (current.kind === "session" && pick.kind === "session") {
      onSession?.({ id: pick.insert.slice("@session:".length), title: pick.label });
      asked++;
      setCompletion(EMPTY);
      loading.value = false;
      return apply(value.slice(0, current.from) + value.slice(current.to).replace(/^ /, ""), current.from);
    }
    const placed = chips?.place(value, current, pick);
    if (placed) return apply(placed.text, placed.caret);
    const finishRef = current.kind === "ref" && !(mode === "descend" && pick.descend);
    const suffix = value.slice(current.to);
    const separator = finishRef && !/^\s/.test(suffix) ? " " : "";
    const next = value.slice(0, current.from) + pick.insert + separator + suffix;
    if (current.kind === "ref") {
      asked++;
      setCompletion(EMPTY);
      loading.value = false;
    }
    apply(next, current.from + pick.insert.length + (finishRef ? 1 : 0));
  };

  // A line that already says what the highlighted row would make it: Enter
  // sends it rather than completing it again.
  const typedChosen = computed(() => completion.value.kind === "slash"
    && completion.value.typed !== undefined
    && items.value[shown.value]?.insert === toValue(text));

  return {
    completion,
    active: shown,
    open,
    typedChosen,
    loading,
    kb,
    move: (delta: number) => {
      const count = items.value.length;
      if (!count) return;
      kb.value = true;
      active.value = (((Math.min(active.value, count - 1) + delta) % count) + count) % count;
    },
    hover: (i: number) => {
      kb.value = false;
      active.value = i;
    },
    dismiss: () => {
      asked++;
      dismissed.value = completion.value;
      loading.value = false;
    },
    accept,
  };
}

// Where the query landed in the label. A fuzzy hit that cannot point at the
// letters it matched reads as the menu ignoring what was typed.
function split(label: string, query: string): [string, string, string] {
  if (!query) return [label, "", ""];
  const i = label.toLowerCase().indexOf(query.toLowerCase());
  if (i < 0) return [label, "", ""];
  return [label.slice(0, i), label.slice(i, i + query.length), label.slice(i + query.length)];
}

export const CompletionMenu = defineComponent({
  name: "CompletionMenu",
  props: {
    id: { type: String, required: true },
    items: { type: Array as PropType<CompletionItem[]>, required: true },
    active: { type: Number, required: true },
    kind: { type: String, required: true },
    query: { type: String, default: "" },
    kb: { type: Boolean, default: true },
    onPick: { type: Function as PropType<(item: CompletionItem) => void>, required: true },
    onHover: { type: Function as PropType<(i: number) => void>, required: true },
  },
  setup(props) {
    let on: HTMLButtonElement | null = null;

    // Arrow keys walk past the eight rows that fit; the list has to follow, or
    // the selection is somewhere below the fold and the menu looks frozen.
    // The key hints sit outside this scrollport on purpose: as a sticky footer
    // inside it they covered the last row while "nearest" still counted it as
    // visible, so the first press past the fold scrolled nothing.
    const follow = () => on?.scrollIntoView({ block: "nearest" });
    watch(() => [props.active, props.items], follow, { flush: "post" });
    onMounted(follow);

    const setActive = (i: number) => (el: Element | ComponentPublicInstance | null) => {
      if (i === props.active) on = el instanceof HTMLButtonElement ? el : null;
    };

    return () => h("div", { class: "menu slashmenu", key: props.kind }, [
      h("div", {
        class: "mlist",
        id: props.id,
        role: "listbox",
        "aria-label": t("common.completions"),
        "data-kb": props.kb ? "" : undefined,
      }, props.items.map((it, i) => {
        const [before, hit, after] = split(it.label, props.query);
        return h("button", {
          key: (it.kind ?? "") + ":" + it.insert,
          id: `${props.id}-${i}`,
          ref: setActive(i),
          class: "mi",
          role: "option",
          "aria-selected": i === props.active,
          "data-on": i === props.active ? "" : undefined,
          "data-kind": it.kind,
          tabindex: -1,
          onMousemove: () => props.onHover(i),
          // mousedown, not click: the textarea must not lose focus first.
          onMousedown: (event: MouseEvent) => {
            event.preventDefault();
            props.onPick(it);
          },
        }, [
          h("span", { class: "dot" }),
          h("span", { class: "tx" }, [
            h("span", { class: "lb" }, [
              before,
              hit ? h("b", { class: "hit" }, hit) : null,
              after,
              it.descend ? h("i", { class: "ah", "aria-hidden": "true" }, [h(StudioIcon, { name: "chevron" })]) : null,
            ]),
            it.hint ? h("span", { class: "ds" }, it.hint) : null,
            ...(it.args ?? []).map((a) => h("span", { class: "ds", key: a.name },
              `${a.name} ${t(a.required ? "chat.required" : "common.optional")}${a.description ? ` · ${a.description}` : ""}`)),
            it.moreArgs ? h("span", { class: "ds" }, t("chat.n_more", { n: it.moreArgs })) : null,
          ]),
          it.kind ? h("span", { class: "rt" }, t(KIND[it.kind] ?? it.kind)) : null,
        ]);
      })),
      h("div", { class: "mkeys" }, t(HINT[props.kind] ?? HINT.slash)),
    ]);
  },
});
export default CompletionMenu;
</script>
