<script setup lang="ts">
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";

withDefaults(defineProps<{
  changes: number;
  focus?: boolean;
  onFiles: () => void;
  onBrowser: () => void;
  onTerminal?: () => void;
  recent?: string[];
  onOpenFile?: (path: string) => void;
}>(), { focus: false, recent: () => [] });
</script>

<template>
  <div class="workbench-launcher" role="group" :aria-label="t('common.start')">
    <p class="workbench-launcher-title">{{ t("workbench.open_in_the_workbench") }}</p>
    <button :autofocus="focus" :aria-label="t('workbench.explorer')" data-action="workbench.launch-files" @click="onFiles">
      <StudioIcon name="folder" />
      <span><b>{{ t("workbench.explorer") }}<small v-if="changes > 0">{{ changes }}</small></b><i>{{ t("workbench.changes_history_and_files") }}</i></span>
    </button>
    <button :aria-label="t('workbench.browser')" data-action="workbench.launch-browser" @click="onBrowser">
      <StudioIcon name="globe" />
      <span><b>{{ t("workbench.browser") }}</b><i>{{ t("workbench.view_web_pages_beside_the") }}</i></span>
    </button>
    <button v-if="onTerminal" :aria-label="t('terminal.name')" data-action="workbench.launch-terminal" @click="onTerminal">
      <StudioIcon name="terminal" />
      <span><b>{{ t("terminal.name") }}</b><i>{{ t("terminal.launch_hint") }}</i></span>
    </button>
    <div v-if="recent.length > 0 && onOpenFile" class="workbench-launcher-recent" role="group" :aria-label="t('workbench.recently_opened')">
      <p>{{ t("workbench.recently_opened") }}</p>
      <button v-for="path in recent" :key="path" data-action="workbench.launch-recent" :data-target="path" :title="path" @click="onOpenFile(path)">
        <StudioIcon name="file" />
        <span>{{ path.slice(path.lastIndexOf("/") + 1) }}</span>
        <i>{{ path.includes("/") ? path.slice(0, path.lastIndexOf("/") + 1) : "" }}</i>
      </button>
    </div>
  </div>
</template>