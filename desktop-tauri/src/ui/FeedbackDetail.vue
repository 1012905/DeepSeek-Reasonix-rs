<script lang="ts">
import { defineComponent, type VNodeChild } from "vue";

const Node = defineComponent({ props: ["value"], setup(props) { return () => props.value as VNodeChild; } });
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { canWithdraw, type FeedbackItem, type FeedbackLimits } from "../port/feedback";
import type { AgentPort } from "../port/port";
import CopyButton from "./CopyButton.vue";
import StatusChips, { snippet, STEP_STATE, steps } from "./feedbackstatus.vue";
import FeedbackThread, { FeedbackReplyBox, FeedbackReviewNote } from "./FeedbackThread.vue";
import FeedbackWithdraw from "./FeedbackWithdraw.vue";
import StudioIcon from "./StudioIcon.vue";

/** One report on its own page: where it stands, the whole exchange, and the box to answer in. */
const props = defineProps<{
  port: AgentPort;
  item: FeedbackItem;
  level: number | null;
  fresh: number;
  limits: FeedbackLimits | null;
  offline: boolean;
  issue: (n: number) => VNodeChild;
  onFile: (url: string) => void;
  onSent: (receipt: string) => void;
  onWithdrawn: () => void;
  onStale: () => void;
  drafts: Map<string, string>;
}>();

const more = ref(false);
const timeline = computed(() => steps(props.item, props.issue));
</script>

<template>
  <section class="fbk-detail" :aria-label="item.titleSnippet" :data-receipt="item.receipt" :data-status="item.status" :data-stale="item.statusUnavailable ? '' : undefined">
    <header class="fbk-detail-hd">
      <div class="fbk-detail-line">
        <h3 class="fbk-detail-title" dir="auto">{{ snippet(item.titleSnippet) }}</h3>
        <StatusChips :item="item" />
        <span v-if="item.otherInstall" class="fbk-chip" data-device="other">{{ t("feedback.from_another_device") }}</span>
        <span class="fbk-detail-fill" />
        <code class="fbk-code">{{ item.receipt }}</code>
        <CopyButton icon-only :text="item.receipt" :label="t('feedback.copy_receipt_number')" />
        <span v-if="canWithdraw(item)" class="fbk-more">
          <button type="button" class="fbk-icon" data-action="feedback.more" :aria-expanded="more" :aria-label="t('feedback.more_actions')" :title="t('feedback.more_actions')" @click="more = !more">
            <StudioIcon name="more" />
          </button>
          <div v-if="more" class="fbk-more-menu">
            <FeedbackWithdraw :port="port" :item="item" :on-done="onWithdrawn" :on-stale="onStale" />
          </div>
        </span>
      </div>
      <ol v-if="!item.statusUnavailable" class="fbk-tl" :aria-label="t('feedback.progress')">
        <li v-for="s in timeline" :key="s.id" :data-state="s.state" :data-tone="s.neutral ? 'neutral' : undefined" :aria-current="s.state === 'current' ? 'step' : undefined">
          <i aria-hidden="true" />
          <span class="sr-only">{{ t(STEP_STATE[s.state]) }}</span>
          <span><Node :value="s.label" /></span>
        </li>
      </ol>
    </header>
    <div class="fbk-detail-body">
      <FeedbackReviewNote :item="item" />
      <FeedbackThread :item="item" :level="level" :fresh="fresh" />
    </div>
    <FeedbackReplyBox :key="item.receipt" :port="port" :item="item" :limit="limits?.replyBytes ?? null" :limits="limits" :offline="offline" :on-sent="onSent" :on-file="onFile" :on-stale="onStale" :drafts="drafts" />
  </section>
</template>
