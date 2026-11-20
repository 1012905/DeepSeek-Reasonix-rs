<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import { drawsHere } from "./nesting";
import MemoryPanel from "../parts/data-memory.vue";

// What the agent remembers about this project. Memory is written per workspace
// as well as per account, so the pane's own port answers it rather than the
// window's.
//
// Two pages draw this block: this one when `at` names "memory", and storage,
// which memory folds into. The column holds one of them at a time, and the
// guard keeps it that way if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("memory");

const port = computed(() => props.settingsProps.localWorkspacePort ?? props.settingsProps.networkPort ?? props.settingsProps.port);
</script>

<template>
  <SectionCard v-if="drawn" :title="t('data.memory')" :hint="t('data.what_the_agent_remembers_and')">
    <Group id="memory" :title="t('data.memory')" :hint="t('settings.what_it_remembers')">
      <MemoryPanel :port="port" :on-error="settingsProps.onError" />
    </Group>
  </SectionCard>
</template>
