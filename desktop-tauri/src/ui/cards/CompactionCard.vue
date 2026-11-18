<script setup lang="ts">
import { computed, defineComponent, h, ref, type PropType, type VNodeChild } from "vue";
import StudioIcon from "../StudioIcon.vue";
import type { Compaction } from "../../port/wire";
import { useStartsOpen } from "../../state/foldpref";
import { Sym } from "../Sym.vue";
import { t } from "../../i18n";
import { FOLD_WHY } from "../../i18n/compaction_why";
import { tokens } from "../../i18n/format";
import { tx } from "../../i18n/rich";

// What sent this fold. The host knows which threshold was reached and how big
// it is; a card that says only "a threshold" leaves a fold at 16% of a declared
// window looking like the host acting on its own.
function why(c: Compaction): string {
  if (c.trigger === "manual") return t("chat.triggered_by_hand");
  if (c.trigger !== "auto") return c.trigger ?? "";
  if (!c.triggerTokens) return t("chat.automatic_a_context_threshold_was");
  const n = tokens(c.triggerTokens);
  if (c.boundary === "capacity") return t("chat.automatic_input_reached_the_windows", { n });
  return t("chat.automatic_input_reached_n", { n });
}

// A digest reads as complete whatever it dropped, so the count of the fold's
// changes it actually carried is stated whether or not anything is missing.
const Coverage = defineComponent({
  name: "CompactionCoverage",
  props: { c: { type: Object as PropType<Compaction>, required: true } },
  setup(props) {
    return () => {
      const required = props.c.coverageRequired ?? 0;
      if (required === 0) return null;
      const missing = props.c.coverageMissing ?? 0;
      const kept = required - missing;
      return h("div", { class: "comp-l" }, [
        h("div", { class: "row" }, [
          h("span", { class: "k" }, h(StudioIcon, { name: "check" })),
          h("span", null, [
            missing === 0
              ? tx("chat.all_n_changes_made_here", { n: h("b", String(required)) })
              : tx("chat.kept_required_changes_are_in", { kept: h("b", String(kept)), required }),
            props.c.coverageBackstopped ? t("chat.gaps_kept_by_the_host") : "",
          ]),
        ]),
        missing > 0
          ? h("div", { class: "row" }, [
            h("span", { class: "x" }, h(StudioIcon, { name: "close" })),
            h("span", null, tx("chat.n_more_are_index_entries", { n: h("b", String(missing)) })),
          ])
          : null,
      ]);
    };
  },
});

const PART: Record<string, string> = {
  system: "common.system_prompt",
  tools: "common.tool_definitions",
  digest: "ext.summary",
  user: "kernel.user_messages",
  reply: "common.model_replies",
  tool_output: "common.tool_output",
};

// What a fold left behind, when that is still a large share of the window.
const Remains = defineComponent({
  name: "CompactionRemains",
  props: { c: { type: Object as PropType<Compaction>, required: true } },
  setup(props) {
    return () => {
      const r = props.c.remains;
      if (!r || r.total <= 0 || r.parts.length === 0) return null;
      const top = r.parts.filter((p) => p.tokens > 0).slice(0, 3).map((p) => `${t(PART[p.name] ?? p.name)} ${tokens(p.tokens)}`).join("、");
      const big = r.largest?.[0];
      return h("div", { class: "comp-l" }, [
        h("div", { class: "row" }, [
          h("span", { class: "k" }, "·"),
          h("span", null, [
            tx("chat.n_still_held_after_the", { n: h("b", tokens(r.total)), top }),
            big && big.tokens > 0
              ? [" · ", big.tool
                ? t("chat.the_largest_is_a_tool", { tool: big.tool, n: tokens(big.tokens) })
                : t("chat.the_largest_is_a_role", { role: t(PART[big.role === "assistant" ? "reply" : big.role === "tool" ? "tool_output" : "user"] ?? big.role), n: tokens(big.tokens) })]
              : null,
          ]),
        ]),
        r.active > 0
          ? h("div", { class: "row" }, [
            h("span", { class: "k" }, "·"),
            h("span", null, t("chat.the_turn_in_progress_holds", { n: tokens(r.active) })),
          ])
          : null,
      ]);
    };
  },
});

// The tail of what has been written, kept to one line: a digest is thousands of
// tokens of headings and lists, and growing the card by all of them would move
// everything below it for the length of the fold.
function lastLine(text: string): string {
  const line = text.trimEnd().split("\n").at(-1)?.trim() ?? "";
  return line.length > 96 ? "…" + line.slice(-96) : line;
}

const Brief = defineComponent({
  name: "CompactionBrief",
  props: { text: { type: String, required: true } },
  setup(props) {
    const start = useStartsOpen("compaction");
    const touched = ref<boolean | null>(null);
    const open = computed(() => touched.value ?? start.value);
    return () => h("details", {
      open: open.value,
      onToggle: (event: Event) => {
        const el = event.currentTarget as HTMLDetailsElement;
        if (el.open !== open.value) touched.value = el.open;
      },
    }, [
      h("summary", null, h("span", { class: "fold" }, t("chat.see_the_digest_it_carries"))),
      h("div", { class: "txt" }, props.text),
    ]);
  },
});

const Content = defineComponent({
  props: { value: { type: null as unknown as PropType<VNodeChild>, default: undefined } },
  setup(props) {
    return () => (props.value === undefined || props.value === null || props.value === false ? null : props.value);
  },
});

const props = defineProps<{ c: Compaction; done: boolean }>();

const before = computed(() => props.c.sourceTokens ?? 0);
const after = computed(() => props.c.projectionTokens ?? 0);
const shrank = computed(() => before.value > 0 && after.value > 0 && after.value < before.value);
const aborted = computed(() => props.done && !props.c.messages);
const cause = computed(() => (aborted.value && props.c.code ? FOLD_WHY[props.c.code] : undefined));
</script>

<template>
  <div class="call" data-k="host" data-light="">
    <div class="g">
      <Sym icon="collapseAll" />
    </div>
    <div class="c">
      <div class="hl">
        <span :class="done ? 'nm' : 'nm shim'">{{ t(!done ? "common.compacting" : aborted ? "chat.compaction_did_not_complete" : "chat.earlier_folded_into_digest") }}</span>
        <span v-if="c.trigger" class="tag">· {{ t(c.trigger === "manual" ? "chat.compaction_manual" : "chat.compaction_auto") }}</span>
      </div>
      <!-- While it runs, the digest's own last line is the progress bar: a fold
           can take a minute, and a placeholder that says nothing cannot be told
           apart from one that has hung. -->
      <div v-if="!done" class="out">
        <div class="comp">
          <div v-if="c.messages || before > 0" class="comp-n">
            <Content v-if="c.messages" :value="tx('chat.folding_n_messages_into_a', { n: h('b', String(c.messages)) })" />
            <template v-else>{{ t("chat.folding_into_a_digest") }}</template>
            <template v-if="before > 0">{{ " · " }}<b>{{ tokens(before) }}</b></template>
          </div>
          <div class="comp-n comp-live">{{ c.summary ? lastLine(c.summary) : " " }}</div>
        </div>
      </div>
      <div v-if="done" class="out">
        <div class="comp">
          <div class="comp-n">
            <template v-if="c.trigger">{{ why(c) }}{{ " · " }}</template>
            <Content v-if="c.messages" :value="tx('chat.folded_n_messages', { n: h('b', String(c.messages)) })" />
            <template v-else>{{ t("chat.this_pass_folded_nothing") }}</template>
            <template v-if="cause">{{ " · " }}{{ t(cause ?? "") }}</template>
            <template v-if="shrank">{{ " · " }}<b>{{ tokens(before) }}</b> → <b>{{ tokens(after) }}</b></template>
          </div>
          <Coverage :c="c" />
          <Remains :c="c" />
          <Brief v-if="c.summary" :text="c.summary" />
        </div>
      </div>
    </div>
  </div>
</template>
