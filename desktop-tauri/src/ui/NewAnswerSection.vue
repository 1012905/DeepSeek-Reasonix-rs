<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onNewAnswerMarkerChange, setShowsNewAnswerMarker, showsNewAnswerMarker } from "../state/newAnswers";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const on = useExternal(onNewAnswerMarkerChange, showsNewAnswerMarker);
</script>

<template>
  <Group id="new-answer-marker" inline :title="t('settings.new_answer_divider')" :hint="t('settings.divider_before_new_answer')">
    <Switch data-action="appearance.new-answer-marker" :on="on" :label="t('settings.new_answer_divider')" :on-click="() => setShowsNewAnswerMarker(!on)" />
  </Group>
</template>