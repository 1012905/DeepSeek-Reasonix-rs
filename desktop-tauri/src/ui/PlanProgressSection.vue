<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onPlanProgressChange, setShowsPlanProgress, showsPlanProgress } from "../state/prefs";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const on = useExternal(onPlanProgressChange, showsPlanProgress);
</script>

<template>
  <Group id="plan-progress" :inline="true" :title="t('plan.show_collapsed_plan_progress')" :hint="t('settings.plan_progress_beside_title')">
    <Switch data-action="appearance.plan-progress" :on="on" :label="t('plan.show_collapsed_plan_progress')" :on-click="() => setShowsPlanProgress(!on)" />
  </Group>
</template>