<script setup lang="ts">
import StudioIcon, { type StudioIconName } from "./StudioIcon.vue";

/** One empty or failed state of a workbench page, as the WB-Parts board draws
 *  them: what it is, why, and what can be done about it. */
withDefaults(defineProps<{
  icon: StudioIconName;
  title: string;
  detail?: string;
  alert?: boolean;
}>(), { alert: false });
</script>

<template>
  <div class="workbench-state" :role="alert ? 'alert' : 'status'">
    <StudioIcon :name="icon" />
    <b>{{ title }}</b>
    <p v-if="detail">{{ detail }}</p>
    <div v-if="$slots.default" class="workbench-state-acts"><slot /></div>
  </div>
</template>