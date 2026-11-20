<script lang="ts">
import type { Item } from "../../state/session";

export interface ElicitProps {
  item: Extract<Item, { t: "ask" }>;
  onAnswer: (itemId: string, id: string, answers: { questionId: string; selected: string[] }[]) => Promise<void>;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { Sym } from "../Sym.vue";
import { t } from "../../i18n";
import { answerSource } from "../source";
import { useViewer } from "../../state/viewer";
import ElicitURLCard from "./ElicitURLCard.vue";

const props = defineProps<ElicitProps>();

const qs = computed(() => props.item.ask.questions);
const origin = computed(() => props.item.ask.origin);
const viewer = useViewer();
const answeredBy = computed(() => answerSource(props.item.by, viewer.value, props.item.said));
const picks = ref<string[][]>(qs.value.map((q) => (q.options.length ? q.default ?? [] : [])));
const text = ref<string[]>(qs.value.map((q) => (q.options.length ? "" : q.default?.[0] ?? "")));
const submitting = ref(false);
const sealed = computed(() => props.item.answered !== undefined);

const given = (i: number) => (qs.value[i].options.length ? picks.value[i] : text.value[i].trim() ? [text.value[i].trim()] : []);
const anyGiven = computed(() => qs.value.some((_, i) => given(i).length > 0));
const at = <T,>(list: T[], i: number, v: T) => list.map((x, k) => (k === i ? v : x));

const toggle = (i: number, label: string) => {
  if (sealed.value) return;
  const cur = picks.value;
  if (!qs.value[i].multi) picks.value = at(cur, i, cur[i][0] === label ? [] : [label]);
  else picks.value = at(cur, i, cur[i].includes(label) ? cur[i].filter((l) => l !== label) : [...cur[i], label]);
};

const typeText = (i: number, event: Event) => {
  text.value = at(text.value, i, (event.target as HTMLInputElement).value);
};

const send = async (answers: string[][]) => {
  if (submitting.value) return;
  submitting.value = true;
  try {
    await props.onAnswer(props.item.id, props.item.ask.id, qs.value.map((q, i) => ({ questionId: q.id, selected: answers[i] })));
  } finally {
    submitting.value = false;
  }
};
</script>

<template>
  <ElicitURLCard v-if="item.ask.origin?.url" :item="item" :on-answer="onAnswer" />
  <div v-else class="call" data-k="ask" :data-prompt="sealed ? 'settled' : 'pending'">
    <div class="g">
      <Sym icon="question" />
    </div>
    <div class="c">
      <div class="hl">
        <span class="nm">{{ t("chat.mcp_server_name_is_asking", { name: origin?.source ?? "" }) }}</span>
        <span class="tag" :data-pending="sealed ? undefined : ''">{{ sealed ? t("chat.handled") : t("chat.needs_you") }}</span>
      </div>
      <div class="out">
        <div class="ask" :data-sealed="sealed ? '' : undefined" :aria-busy="submitting">
          <div class="ask-pane" data-on="">
            <div class="ask-hint">{{ t("chat.this_request_comes_from_an") }}</div>
            <div v-if="origin?.message" class="ask-q">{{ origin.message }}</div>
            <div v-if="origin?.note && !sealed" class="ask-hint">{{ t("chat.the_last_answer_was_not", { note: origin.note }) }}</div>
          </div>
          <div v-for="(q, i) in qs" :key="q.id" class="ask-pane" data-on="">
            <div class="ask-q">{{ q.header || q.id }}</div>
            <div v-if="q.prompt && q.prompt !== q.header" class="ask-hint">{{ q.prompt }}</div>
            <div v-if="q.options.length > 0" class="opts" :aria-label="q.header || q.id">
              <button
                v-for="o in q.options"
                :key="o.label"
                class="opt"
                :data-multi="q.multi ? '' : undefined"
                :data-on="((sealed ? item.answered?.[i] : picks[i])?.includes(o.label) ?? false) ? '' : undefined"
                :aria-pressed="(sealed ? item.answered?.[i] : picks[i])?.includes(o.label) ?? false"
                :disabled="sealed"
                @click="toggle(i, o.label)"
              >
                <span class="mark" />
                <span class="txt"><span class="lb">{{ o.label }}</span></span>
              </button>
            </div>
            <div v-else class="other-wrap" data-on="">
              <input
                :aria-label="q.header || q.id"
                :value="sealed ? (item.answered?.[i] ?? []).join('') : text[i]"
                :readonly="sealed"
                :placeholder="t('chat.fill_this_in')"
                @input="typeText(i, $event)"
              />
            </div>
          </div>
          <div v-if="sealed" class="ask-done">
            <b v-if="item.answeredElsewhere">{{ answeredBy || t("chat.handled_in_another_window_refer") }}</b>
            <b v-else-if="item.answered?.some((a) => a.length)">{{ t("chat.sent_to_name", { name: origin?.source ?? "" }) }}</b>
            <b v-else>{{ t("chat.declined") }}</b>
          </div>
          <div v-else class="ask-foot">
            <button class="dismiss" data-action="ask.answer" data-value="none" :disabled="submitting" @click="send(qs.map(() => []))">
              {{ t("chat.decline") }}
            </button>
            <button class="btn" data-primary data-action="ask.answer" data-value="chosen" :disabled="!anyGiven || submitting" @click="send(qs.map((_, i) => given(i)))">
              {{ submitting ? t("common.submitting") : t("chat.submit") }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
