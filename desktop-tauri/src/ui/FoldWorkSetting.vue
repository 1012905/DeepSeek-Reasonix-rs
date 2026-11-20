<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import { saveFoldWorkPref, useFoldsTurnWork } from "../state/foldwork";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const props = defineProps<{ port: AgentPort }>();
const on = useFoldsTurnWork(props.port);
const error = ref("");
const toggle = () => {
  error.value = "";
  saveFoldWorkPref(props.port, !on.value).catch((e) => (error.value = reason(e)));
};
</script>

<template>
  <Group id="fold-work" inline :title="t('settings.fold_the_work_in_between')" :hint="t('settings.tool_calls_and_thinking_fold')">
    <Switch data-action="chrome.fold-work" :on="on" :label="t('settings.fold_the_work_inside_a')" :on-click="toggle" />
    <div v-if="error" class="why">{{ error }}</div>
  </Group>
</template>