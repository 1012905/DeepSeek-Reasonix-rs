<script setup lang="ts">
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";

/** One turn's intermediate work under a single line. The body is always
 *  rendered, closed or not: find, copy and the graph's landing reach into it,
 *  and opening it must not remount what was already drawn. */
const props = defineProps<{
  open: boolean;
  onOpen: (next: boolean) => void;
  calls: number;
  failures: number;
  words: number;
}>();

const toggle = (e: Event) => {
  const el = e.currentTarget as HTMLDetailsElement;
  if (el.open !== props.open) props.onOpen(el.open);
};
</script>

<template>
  <details class="activity-group work-fold" :data-failed="failures ? '' : undefined" :open="open" @toggle="toggle">
    <summary>
      <StudioIcon :name="failures ? 'warning' : 'check'" class="activity-status-icon" />
      <span class="activity-title">{{ t("chat.work_in_between") }}</span>
      <span v-if="failures > 0" class="activity-errors">{{ t("chat.n_failed", { n: failures }) }}</span>
      <span class="activity-count">{{ calls > 0 ? t("chat.count_actions", { count: calls }) : t("chat.thinking_and_notes") }}</span>
      <span v-if="words > 0 && calls > 0" class="activity-count">{{ t("chat.notes_count", { count: words }) }}</span>
      <StudioIcon name="down" class="activity-fold-icon" />
    </summary>
    <div class="activity-body work-fold-body"><slot /></div>
  </details>
</template>