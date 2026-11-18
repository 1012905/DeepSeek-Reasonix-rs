<script setup lang="ts">
import { t } from "../../i18n";
import type { PaneView } from "../../state/paneview";
import { ariaChord, chord } from "../keys";
import StudioIcon from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";

const props = defineProps<{
  view: PaneView;
  rows: number;
  surfaces: number;
  onView: (to: PaneView) => void;
}>();

const press = (to: PaneView) => props.onView(props.view === to ? "flow" : to);
</script>

<template>
  <TooltipButton
    v-if="rows > 0"
    type="button"
    class="thbtn view-action top-action"
    data-action="pane.view"
    data-value="analysis"
    :aria-pressed="view === 'analysis'"
    :aria-label="t('common.run_analysis')"
    :aria-keyshortcuts="ariaChord('2')"
    :title="view === 'analysis' ? t('app.close_run_analysis') : t('app.open_run_analysis')"
    :shortcut="chord('2')"
    @click="press('analysis')"
  >
    <StudioIcon name="activity" />
    <span class="view-label">{{ t("common.run_analysis") }}</span>
  </TooltipButton>
  <TooltipButton
    type="button"
    class="thbtn view-action"
    data-action="pane.view"
    data-value="browser"
    :aria-pressed="view === 'browser'"
    :aria-label="t('common.workbench')"
    :aria-keyshortcuts="ariaChord('3')"
    :title="view === 'browser' ? t('app.close_workbench') : t('app.open_workbench')"
    :shortcut="chord('3')"
    @click="press('browser')"
  >
    <StudioIcon name="panel" />
    <span v-if="surfaces > 0" class="view-count">{{ surfaces }}</span>
  </TooltipButton>
</template>
