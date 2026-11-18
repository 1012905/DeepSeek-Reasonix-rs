<script setup lang="ts">
import { computed, ref } from "vue";
import { host } from "../../port/host";
import { t } from "../../i18n";
import { Sym } from "../Sym.vue";
import { answerSource } from "../source";
import { useViewer } from "../../state/viewer";
import type { ElicitProps } from "./ElicitCard.vue";

const props = defineProps<ElicitProps>();

const viewer = useViewer();
const origin = computed(() => props.item.ask.origin!);
const url = computed(() => externalURL(origin.value.url!));
const sealed = computed(() => props.item.answered !== undefined);
const answeredBy = computed(() => answerSource(props.item.by, viewer.value, props.item.said));
const busy = ref(false);
const failure = ref(false);

const open = async () => {
  if (sealed.value || busy.value || !url.value) return;
  failure.value = false;
  busy.value = true;
  try {
    await host().openExternal(origin.value.url!);
  } catch {
    failure.value = true;
  } finally {
    busy.value = false;
  }
};
const answer = async (action: string) => {
  if (sealed.value || busy.value) return;
  busy.value = true;
  try {
    await props.onAnswer(props.item.id, props.item.ask.id, [{ questionId: props.item.ask.questions[0].id, selected: [action] }]);
  } finally {
    busy.value = false;
  }
};
const outcome = computed(() => props.item.answered?.[0]?.[0]);

function externalURL(raw: string): URL | null {
  try {
    const url = new URL(raw);
    return (url.protocol === "http:" || url.protocol === "https:") && !url.username && !url.password ? url : null;
  } catch {
    return null;
  }
}
</script>

<template>
  <div class="call" data-k="ask" :data-prompt="sealed ? 'settled' : 'pending'">
    <div class="g"><Sym icon="external" /></div>
    <div class="c">
      <div class="hl"><span class="nm">{{ t("chat.mcp_server_name_is_asking_2", { name: origin.source }) }}</span></div>
      <div class="out">
        <div class="ask" :data-sealed="sealed ? '' : undefined" :aria-busy="busy">
          <div class="ask-pane" data-on="">
            <div class="ask-hint">{{ t("chat.complete_the_interaction_in_your") }}</div>
            <div v-if="origin.message" class="ask-q">{{ origin.message }}</div>
            <div v-if="url" class="ask-q"><b>{{ origin.urlHost || url.hostname }}</b></div>
            <div class="ask-hint" style="overflow-wrap: anywhere">{{ origin.url }}</div>
            <div v-if="url && (url.protocol !== 'https:' || url.hostname.includes('xn--'))" class="ask-hint" role="note">{{ t("chat.check_the_full_address_this") }}</div>
            <div v-if="origin.urlLocal" class="ask-hint" role="note">{{ t("chat.this_link_targets_this_device") }}</div>
            <div v-if="failure || !url" class="ask-hint" role="alert">{{ t("chat.could_not_open_the_browser") }}</div>
          </div>
          <div v-if="sealed" class="ask-done"><b>{{ item.answeredElsewhere ? answeredBy || t("chat.handled_in_another_window_refer") : outcome === "accept" ? t("chat.completion_confirmed") : outcome === "cancel" ? t("common.cancelled") : t("chat.declined") }}</b></div>
          <div v-else class="ask-foot">
            <button class="dismiss" data-action="ask.answer" data-value="cancel" :disabled="busy" @click="answer('cancel')">{{ t("common.cancel") }}</button>
            <button class="dismiss" data-action="ask.answer" data-value="decline" :disabled="busy" @click="answer('decline')">{{ t("chat.decline") }}</button>
            <button class="btn" data-action="external.open" :disabled="busy || !url" @click="open">{{ t("chat.open_browser") }}</button>
            <button class="btn" data-primary data-action="ask.answer" data-value="accept" :disabled="busy" @click="answer('accept')">{{ t("chat.completed_continue") }}</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
