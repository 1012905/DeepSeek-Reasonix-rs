<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import { loadLevelNoticePref, saveLevelNoticePref, useShowsLevelNotice } from "../state/levelnotices";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const props = defineProps<{ port: AgentPort }>();
const on = useShowsLevelNotice(props.port);
const error = ref("");
watch(() => props.port, (port) => { void loadLevelNoticePref(port); }, { immediate: true });
const toggle = () => {
  error.value = "";
  saveLevelNoticePref(props.port, !on.value).catch((e) => (error.value = reason(e)));
};
</script>

<template>
  <Group id="feedback-level-notices" inline :title="t('feedback.level_notices')" :hint="t('settings.notice_when_feedback_adopted')">
    <Switch data-action="chrome.feedback-level-notices" :on="on" :label="t('feedback.tell_me_when_my_level')" :on-click="toggle" />
    <div v-if="error" class="why">{{ error }}</div>
  </Group>
</template>