<script lang="ts">
/** The find bar over one pane's transcript.
 *
 *  `host` is the scrolling element the transcript lives in, not the bar's own
 *  container: the bar floats over the transcript's top edge and reads that
 *  element's rows, and the pane owns both, so passing the wrong one would put
 *  the bar over a different scroll than the one it searches. */
export interface FindProps {
  open: boolean;
  onClose: () => void;
  /** The transcript to search, as rendered. */
  host: HTMLElement | null;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, toRef, watch } from "vue";
import { t } from "../../i18n";
import { WholeWord } from "lucide-vue-next";
import StudioIcon from "../StudioIcon.vue";
import { useFindPaint } from "../findland";
import { useFindBar, useFindReveal } from "./findBar";

const props = defineProps<FindProps>();

const host = toRef(props, "host");
const bar = useFindBar(() => props.host, () => props.open, (e) => props.onError(e));

// The painter and the counter read the same query through the same finder, so a
// hit the box counts is one the highlight can draw.
useFindPaint(host, () => !props.open, () => bar.text.value, () => bar.current.value?.id ?? null, () => bar.current.value?.nth ?? 0, () => bar.mode.value);
useFindReveal(() => props.host, () => bar.at.value, () => bar.text.value);

const box = ref<HTMLInputElement | null>(null);

watch(() => props.open, (on) => {
  if (!on) {
    bar.reset();
    return;
  }
  // Opening asks the question again, so the field takes the caret and the text
  // it already holds is selected — retyping replaces it instead of adding to it.
  void nextTick(() => {
    box.value?.focus();
    box.value?.select();
  });
}, { immediate: true });

const asked = computed(() => bar.query.value.trim().length > 0);
// Without a transcript there is nothing to have searched: saying "no matches"
// there would blame the query for a pane that has not drawn one yet.
const missed = computed(() => asked.value && !!props.host && bar.total.value === 0);
const count = computed(() =>
  bar.total.value > 0 ? `${bar.position.value + 1} / ${bar.total.value}${bar.capped.value ? "+" : ""}` : missed.value ? t("chat.no_matches") : "",
);
// Whole word rides the regex path (see reading()), so the advice for it is the
// advice for a regular expression: turn it off and the plain search runs.
const advice = computed(() =>
  bar.caseSensitive.value ? t("chat.find_try_without_case") : bar.wholeWord.value ? t("chat.find_try_without_regex") : "",
);
const hint = computed(() => (missed.value ? [t("chat.find_searched_everything"), advice.value].filter(Boolean).join(" · ") : ""));

function key(event: KeyboardEvent) {
  // A press that is still composing belongs to the input method, not to us.
  if (event.isComposing) return;
  if (event.key === "Enter" || event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    bar.step(event.key === "ArrowUp" || (event.key === "Enter" && event.shiftKey) ? -1 : 1);
  } else if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    props.onClose();
  }
}
</script>

<template>
  <div v-if="open" class="findhost">
    <div class="tfindbox">
      <div class="tfind" role="search" :data-missed="missed ? '' : undefined">
        <StudioIcon name="search" />
        <input ref="box" type="search" data-window-keys="" aria-keyshortcuts="Enter Shift+Enter ArrowDown ArrowUp Escape"
          data-action-change="transcript.find" data-action-keydown="transcript.find" data-value="query" :value="bar.query.value"
          :placeholder="t('common.find_in_this_conversation')" :aria-label="t('common.find_in_this_conversation')"
          @input="bar.ask(($event.target as HTMLInputElement).value)" @keydown="key" />
        <span class="n" :data-empty="missed ? '' : undefined" aria-live="polite">{{ count }}</span>
        <button class="st" type="button" data-action="transcript.find" data-value="prev" :disabled="bar.total.value === 0"
          :aria-label="t('chat.previous_match')" :title="t('chat.previous_match')" @click="bar.step(-1)">
          <StudioIcon name="up" />
        </button>
        <button class="st" type="button" data-action="transcript.find" data-value="next" :disabled="bar.total.value === 0"
          :aria-label="t('chat.next_match')" :title="t('chat.next_match')" @click="bar.step(1)">
          <StudioIcon name="down" />
        </button>
        <span class="sep" aria-hidden="true" />
        <button class="mode-toggle" type="button" data-action="transcript.find" data-value="case"
          :aria-pressed="bar.caseSensitive.value" :aria-label="t('common.match_case')" :title="t('common.match_case')"
          @click="bar.setCase(!bar.caseSensitive.value)">
          <StudioIcon name="matchCase" />
        </button>
        <button class="mode-toggle" type="button" data-action="transcript.find" data-value="word"
          :aria-pressed="bar.wholeWord.value" :aria-label="t('workbench.whole_word')" :title="t('workbench.whole_word')"
          @click="bar.setWholeWord(!bar.wholeWord.value)">
          <WholeWord :size="14" aria-hidden="true" focusable="false" />
        </button>
        <button class="st" type="button" data-action="transcript.find" data-value="close"
          :aria-label="t('chat.close_find')" :title="t('chat.close_find')" @click="onClose()">
          <StudioIcon name="close" />
        </button>
      </div>
      <div v-if="missed" class="tfind-hint">{{ hint }}</div>
    </div>
  </div>
</template>
