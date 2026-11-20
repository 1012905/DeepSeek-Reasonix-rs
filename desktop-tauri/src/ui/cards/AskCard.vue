<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, watch, watchEffect } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { AskReason } from "../../port/session";
import type { Item } from "../../state/session";
import { answerSource } from "../source";
import { useViewer } from "../../state/viewer";
import { useIme } from "../ime";
import LazyMarkdown from "../LazyMarkdown.vue";
import { Sym } from "../Sym.vue";
import { clock, useAskCountdown } from "./useAskCountdown";
import AskFreeText from "./AskFreeText.vue";
import { clearAskDraft, readAskDraft, writeAskDraft } from "../../state/askdraft";

const ADVANCE_MS = 220;

const props = defineProps<{
  item: Extract<Item, { t: "ask" }>;
  onAnswer: (itemId: string, id: string, answers: { questionId: string; selected: string[] }[]) => Promise<void>;
  onEngage?: (id: string) => Promise<void>;
  refine?: (draft: string, signal?: AbortSignal) => Promise<string>;
}>();

const qs = computed(() => props.item.ask.questions);
const viewer = useViewer();
const answeredBy = computed(() => answerSource(props.item.by, viewer.value, props.item.said));
const saved = readAskDraft(props.item.ask.id, qs.value.length);
const tab = ref(saved.tab);
const picks = ref<string[][]>(saved.picks);
// The wire carries selections as free strings and the kernel joins them into
// the tool result untouched, so "其他" is a label like any other.
const other = ref<string[]>(saved.other);
const otherOn = ref<boolean[]>(saved.otherOn);
// A pick's note is its own text: an Other answer left behind by a pick is not a
// note, and returning to Other finds it where it was.
const note = ref<string[]>(saved.note);
const submitting = ref(false);
// Every request is a new object, so asking for the same pane twice still moves focus.
const focusTo = ref<{ q: number } | null>(null);
const boxes: (HTMLInputElement | null)[] = [];
const panes: (HTMLDivElement | null)[] = [];
const timers: number[] = [];
onScopeDispose(() => timers.forEach((id) => window.clearTimeout(id)));
const ime = useIme();
const sealed = computed(() => props.item.answered !== undefined);
watchEffect(() => {
  const id = props.item.ask.id;
  if (sealed.value) {
    clearAskDraft(id);
    return;
  }
  writeAskDraft(id, { tab: tab.value, picks: picks.value, other: other.value, otherOn: otherOn.value, note: note.value });
});
let engagement: "idle" | "pending" | "engaged" = "idle";
const engagementError = ref("");
const kept = ref(false);
const skipIn = useAskCountdown(() => props.item.skipAt, () => !sealed.value && !kept.value);
const engage = () => {
  if (sealed.value || !props.onEngage || engagement !== "idle") return;
  engagement = "pending";
  void props.onEngage(props.item.ask.id).then(() => {
    engagement = "engaged";
    kept.value = true;
    engagementError.value = "";
  }).catch((err: unknown) => {
    engagement = "idle";
    engagementError.value = reason(err);
  });
};
const chosen = computed(() => props.item.answered ?? picks.value);

// A single-choice pick keeps the box open as a note that rides with it, so an
// option asking for more ("paste the output") has somewhere to take it.
const noting = (i: number) => !qs.value[i].multi && !otherOn.value[i] && picks.value[i].length > 0;
const free = (i: number) => (otherOn.value[i] ? other.value[i].trim() : noting(i) ? note.value[i].trim() : "");
const selected = (i: number) => (free(i) ? [...picks.value[i], free(i)] : picks.value[i]);
const answered = (i: number) => (sealed.value ? (chosen.value[i]?.length ?? 0) > 0 : selected(i).length > 0);
const left = computed(() => qs.value.reduce((n, _, i) => n + (answered(i) ? 0 : 1), 0));
// A sealed card may have been answered by another client, so what counts as
// free text is whatever came back that no option offered.
const sealedFree = (i: number) => {
  const offered = new Set(qs.value[i].options.map((o) => o.label));
  return (chosen.value[i] ?? []).filter((v) => !offered.has(v)).join("、");
};
const freeShown = (i: number) => (sealed.value ? sealedFree(i) : noting(i) ? note.value[i] : other.value[i]);
const customOption = (i: number) => qs.value[i].options.find((option) => isOtherOption(option.label));
const sealedPick = (i: number) =>
  qs.value[i].options.some((o) => !isOtherOption(o.label) && (chosen.value[i] ?? []).includes(o.label));
const noteShown = (i: number) => (sealed.value ? !qs.value[i].multi && sealedPick(i) && !!sealedFree(i) : noting(i));
const customChosen = (i: number) => {
  const offered = customOption(i);
  return sealed.value
    ? (!!sealedFree(i) && !noteShown(i)) || (!!offered && (chosen.value[i] ?? []).includes(offered.label))
    : otherOn.value[i];
};

const at = <T,>(list: T[], i: number, v: T) => list.map((x, k) => (k === i ? v : x));

// The next question still waiting for an answer after `from`, wrapping, or -1.
const nextOpen = (from: number, done = -1) => {
  for (let k = 1; k < qs.value.length; k++) {
    const j = (from + k) % qs.value.length;
    if (j !== done && !answered(j)) return j;
  }
  return -1;
};

// Stepping forward is positional: only the last question looks for one still
// waiting, so a question already answered is still visited on the way.
const stepTo = (from: number, done = -1) => (from < qs.value.length - 1 ? from + 1 : nextOpen(from, done));

const toggle = (qi: number, label: string) => {
  if (sealed.value) return;
  const cur = picks.value;
  if (!qs.value[qi].multi) picks.value = at(cur, qi, [label]);
  else picks.value = at(cur, qi, cur[qi].includes(label) ? cur[qi].filter((l) => l !== label) : [...cur[qi], label]);
  if (!qs.value[qi].multi) {
    otherOn.value = at(otherOn.value, qi, false);
    // One pick answers a single-choice question, so the card moves on by
    // itself; the pause lets the mark land before the pane changes.
    const next = stepTo(qi, qi);
    if (next >= 0) timers.push(window.setTimeout(() => { if (tab.value === qi) tab.value = next; }, ADVANCE_MS));
  }
};

const toggleOther = (qi: number) => {
  if (sealed.value) return;
  const on = !otherOn.value[qi];
  otherOn.value = at(otherOn.value, qi, on);
  if (on) focusTo.value = { q: qi };
  if (on && !qs.value[qi].multi) picks.value = at(picks.value, qi, []);
};

const enterBox = (qi: number) => {
  if (sealed.value || noting(qi) || otherOn.value[qi]) return;
  otherOn.value = at(otherOn.value, qi, true);
  if (!qs.value[qi].multi) picks.value = at(picks.value, qi, []);
};

const type = (qi: number, text: string) => {
  if (noting(qi)) {
    note.value = at(note.value, qi, text);
    return;
  }
  other.value = at(other.value, qi, text);
  if (text && !otherOn.value[qi]) {
    otherOn.value = at(otherOn.value, qi, true);
    if (!qs.value[qi].multi) picks.value = at(picks.value, qi, []);
  }
};

// A pane is display:none until it is the current one, so focus moves after the
// render that shows it: into its open free-text or note box, else its first option.
watch(focusTo, async (to) => {
  if (!to) return;
  await nextTick();
  const box = boxes[to.q];
  if ((otherOn.value[to.q] || noting(to.q)) && box) box.focus();
  else panes[to.q]?.querySelector<HTMLElement>("button.opt")?.focus();
}, { flush: "post" });

const send = async (answers: string[][]) => {
  if (submitting.value) return;
  submitting.value = true;
  try {
    await props.onAnswer(props.item.id, props.item.ask.id, qs.value.map((q, i) => ({ questionId: q.id, selected: answers[i] })));
  } finally {
    submitting.value = false;
  }
};

// Enter in the free-text box does what the primary button would, so a typed
// answer never needs the mouse to go on.
const onEnter = () => {
  if (submitting.value) return;
  if (stepTo(tab.value, tab.value) >= 0) {
    if (!answered(tab.value)) return;
    const next = stepTo(tab.value);
    tab.value = next;
    focusTo.value = { q: next };
  } else if (left.value === 0) {
    void send(qs.value.map((_, i) => selected(i)));
  }
};

function isOtherOption(label: string): boolean {
  return /^\s*(?:其他|其它|other)(?:\s|[（(—\-:：]|$)/i.test(label);
}

// Exhaustive on purpose: a third reason would be a decision with a different
// owner, and adding one must fail this compile rather than render as a question.
function reasonLabel(kind: AskReason): string {
  switch (kind) {
    case "user_decision":
      return t("chat.need_your_decision");
    case "missing_value":
      return t("chat.need_more_information_from_you");
    default: {
      const unhandled: never = kind;
      return unhandled;
    }
  }
}

// A batch may mix the two; naming both is more honest than picking one.
function askHeading(questions: { reason?: AskReason }[]): string {
  const kinds = new Set<AskReason>(questions.map((q) => q.reason ?? "user_decision"));
  return [...kinds].map(reasonLabel).join(" · ");
}
</script>

<template>
  <div class="call" data-k="ask" :data-prompt="sealed ? 'settled' : 'pending'">
    <div class="g">
      <Sym icon="question" />
    </div>
    <div class="c">
      <div class="hl">
        <span class="nm">{{ askHeading(qs) }}</span>
        <span class="tag" :data-pending="sealed ? undefined : ''">{{ sealed ? t("chat.handled") : t("chat.needs_you") }}</span>
      </div>
      <div class="out">
        <div
          class="ask"
          :data-sealed="sealed ? '' : undefined"
          :aria-busy="submitting"
          @pointerdown.capture="engage"
          @keydown.capture="engage"
          @input.capture="engage"
          @wheel.capture="engage"
          @touchmove.capture="engage"
        >
          <div v-if="!sealed && engagementError" class="err" role="alert">{{ engagementError }}</div>
          <div v-if="qs.length > 1" class="ask-tabs" role="tablist">
            <button
              v-for="(q, i) in qs"
              :key="q.id"
              class="ask-tab"
              role="tab"
              :aria-selected="i === tab"
              :data-answered="answered(i) ? '' : undefined"
              @click="tab = i"
            >
              {{ q.header || t("chat.question_n", { n: i + 1 }) }}
              <i class="dot" />
            </button>
          </div>
          <div
            v-for="(q, i) in qs"
            :key="q.id"
            class="ask-pane"
            :data-on="i === tab ? '' : undefined"
            :ref="(el) => (panes[i] = el as HTMLDivElement | null)"
          >
            <div class="ask-q"><LazyMarkdown :text="q.prompt" /></div>
            <div class="ask-hint">{{ t(q.multi ? "chat.multiple_allowed" : "common.pick_one") }}</div>
            <div class="opts" :aria-label="q.prompt">
              <button
                v-for="o in q.options.filter((o) => !isOtherOption(o.label))"
                :key="o.label"
                class="opt"
                :data-multi="q.multi ? '' : undefined"
                :data-on="chosen[i]?.includes(o.label) ? '' : undefined"
                :aria-pressed="chosen[i]?.includes(o.label) ?? false"
                :disabled="sealed"
                @click="toggle(i, o.label)"
              >
                <span class="mark" />
                <span class="txt">
                  <span class="lb">{{ o.label }}</span>
                  <span v-if="o.description" class="ds">{{ o.description }}</span>
                </span>
              </button>
              <button
                class="opt opt-other"
                :data-multi="q.multi ? '' : undefined"
                :data-on="customChosen(i) ? '' : undefined"
                :aria-pressed="customChosen(i)"
                :disabled="sealed"
                @click="toggleOther(i)"
              >
                <span class="mark" />
                <span class="txt">
                  <span class="lb">{{ customOption(i)?.label ?? t("chat.something_else_i_will_write") }}</span>
                  <span v-if="customOption(i)?.description" class="ds">{{ customOption(i)?.description }}</span>
                </span>
              </button>
            </div>
            <div class="other-wrap" :data-on="customChosen(i) || noteShown(i) ? '' : undefined">
              <AskFreeText
                :input-ref="(el) => (boxes[i] = el as HTMLInputElement | null)"
                :label="noteShown(i) ? t('chat.note_optional') : (customOption(i)?.label ?? t('chat.something_else_i_will_write'))"
                :value="freeShown(i)"
                :read-only="sealed"
                :placeholder="noteShown(i) ? t('chat.add_a_note_optional_it') : t('chat.or_type_your_own_answer')"
                :on-change="(text) => type(i, text)"
                :on-focus="() => enterBox(i)"
                :on-enter="onEnter"
                :is-ime="ime.isIme"
                :ime-handlers="ime.handlers"
                :refine="refine"
              />
            </div>
          </div>
          <div v-if="sealed" class="ask-done" :data-recorded="item.recorded !== undefined ? '' : undefined">
            <template v-if="item.recorded !== undefined">{{ item.recorded }}</template>
            <b v-else-if="item.timedOut">{{ t("chat.nobody_answered_in_time_this") }}</b>
            <b v-else-if="item.answeredElsewhere">{{ answeredBy || t("chat.handled_in_another_window_refer") }}</b>
            <template v-else>
              <span v-for="(q, i) in qs" :key="q.id">{{ i > 0 ? "　·　" : "" }}<b>{{ q.header || t("chat.question_n", { n: i + 1 }) }}：</b>{{ chosen[i]?.length ? chosen[i].join("、") : t("chat.unanswered") }}</span>
            </template>
          </div>
          <div v-if="!sealed" class="ask-foot">
            <span v-if="skipIn !== null" class="ask-skip" role="timer" aria-live="off">
              {{ t("chat.this_question_is_skipped_in", { time: skipIn < 60 ? t("common.n_s", { n: skipIn }) : clock(skipIn) }) }}
            </span>
            <button class="dismiss" data-action="ask.answer" data-value="none" :disabled="submitting" @click="send(qs.map(() => []))">
              {{ t("chat.skip_the_choice_and_reply") }}
            </button>
            <button v-if="qs.length > 1 && tab > 0" class="dismiss" data-action="ask.step" data-value="prev" :disabled="submitting" @click="tab = tab - 1">
              {{ t("chat.previous") }}
            </button>
            <button
              v-if="stepTo(tab, tab) >= 0"
              class="btn"
              data-primary
              data-action="ask.step"
              data-value="next"
              :disabled="!answered(tab) || submitting"
              @click="tab = stepTo(tab)"
            >
              {{ t("chat.next_i_n", { i: tab + 1, n: qs.length }) }}
            </button>
            <button v-else class="btn" data-primary data-action="ask.answer" data-value="chosen" :disabled="left > 0 || submitting" @click="send(qs.map((_, i) => selected(i)))">
              {{ submitting ? t("common.submitting") : t("common.confirm") }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
