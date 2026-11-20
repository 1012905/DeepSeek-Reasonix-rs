<script setup lang="ts">
import { t } from "../i18n";
import type { FeedbackItem } from "../port/feedback";
import { FEEDBACK_GROUPS, inGroup, type FeedbackGroup } from "./feedbackgroups";

const props = defineProps<{
  items: FeedbackItem[];
  fresh: Record<string, number>;
  group: FeedbackGroup;
  onPick: (to: FeedbackGroup) => void;
}>();

const LABEL: Record<FeedbackGroup, string> = {
  all: "common.all", fresh: "feedback.new_replies", input: "feedback.needs_your_reply", open: "feedback.open", done: "feedback.closed",
};

/** One chip per group, each carrying how many reports it holds. */
const countFor = (g: FeedbackGroup) => props.items.filter((i) => inGroup(i, g, props.fresh[i.receipt] ?? 0)).length;
</script>

<template>
  <div class="fbk-filter" role="group" :aria-label="t('feedback.filter_by_progress')">
    <button v-for="g in FEEDBACK_GROUPS" :key="g" type="button" class="fbk-filter-chip" data-action="feedback.filter" :data-value="g" :data-live="g === 'fresh' && countFor(g) > 0 ? '' : undefined" :aria-pressed="group === g" @click="onPick(g)">{{ t(LABEL[g]) }}<small>{{ countFor(g) }}</small></button>
  </div>
</template>