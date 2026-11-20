<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import { saveChangedFilesPref, useShowsChangedFiles } from "../state/changedfiles";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const props = defineProps<{ port: AgentPort }>();
const on = useShowsChangedFiles(props.port);
const error = ref("");
const toggle = () => {
  error.value = "";
  saveChangedFilesPref(props.port, !on.value).catch((e) => (error.value = reason(e)));
};
</script>

<template>
  <Group id="changed-files" inline :title="t('settings.file_changes')" :hint="t('settings.show_n_files_changed_above_input')">
    <Switch data-action="chrome.changed-files" :on="on" :label="t('settings.show_file_changes_above_the')" :on-click="toggle" />
    <div v-if="error" class="why">{{ error }}</div>
  </Group>
</template>