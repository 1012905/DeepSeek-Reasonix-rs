<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { FEEDBACK_CODE, canWithdraw, type FeedbackItem } from "../port/feedback";
import { HttpError, type AgentPort } from "../port/port";

// Taking a report back closes its public issue, so the person is told that
// before anything is sent. Only the service knows whether the report already
// has an outcome; a refusal for that reason reads the list again.
const props = defineProps<{ port: AgentPort; item: FeedbackItem; onDone: () => void; onStale: () => void }>();

const asking = ref(false);
const busy = ref(false);
const error = ref("");

const withdraw = async () => {
  busy.value = true;
  error.value = "";
  try {
    await props.port.withdrawFeedback(props.item.receipt);
    props.onDone();
  } catch (e) {
    error.value = reason(e);
    busy.value = false;
    if (e instanceof HttpError && e.reason?.code === FEEDBACK_CODE.notWithdrawable) props.onStale();
  }
};
</script>

<template>
  <template v-if="canWithdraw(item)">
    <div v-if="!asking" class="fbk-withdraw">
      <button type="button" class="btn sm" data-action="feedback.withdraw-open" :data-target="item.receipt" @click="asking = true">{{ t("feedback.withdraw_this_report") }}</button>
      <p v-if="error" class="fbk-hint" role="alert">{{ error }}</p>
    </div>
    <div v-else class="fbk-withdraw" data-stage="confirm" :aria-busy="busy">
      <p class="fbk-hint">{{ item.issueNumber ? t("feedback.once_withdrawn_the_maintainers_stop_2", { n: item.issueNumber }) : t("feedback.once_withdrawn_the_maintainers_stop") }}</p>
      <div class="fbk-withdraw-acts">
        <button type="button" class="btn sm" data-action="feedback.withdraw-cancel" :data-target="item.receipt" :disabled="busy" @click="asking = false">{{ t("common.cancel") }}</button>
        <button type="button" class="btn sm danger" data-action="feedback.withdraw-confirm" :data-target="item.receipt" :disabled="busy" @click="withdraw">{{ t("feedback.withdraw") }}</button>
      </div>
      <p v-if="error" class="fbk-hint" role="alert">{{ error }}</p>
    </div>
  </template>
</template>