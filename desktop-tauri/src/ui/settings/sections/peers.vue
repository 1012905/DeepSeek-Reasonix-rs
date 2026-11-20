<script setup lang="ts">
import type { SettingsProps } from "../../SettingsProps";
import SectionCard from "../../SectionCard.vue";
import PeersSection from "../../PeersSection.vue";
import { drawsHere } from "./nesting";
import { t } from "../../../i18n";

// Messages between the person's own sessions. The whole block is one component
// because the switches and the released projects are two views of one file:
// a save in either has to reach both, and usePeersSettings is what publishes it.
//
// Two pages draw these blocks: this one when `at` names "peers", and session,
// which peers folds into. The column holds one of them at a time; ./nesting is
// what keeps it one if a page ever nests a section it already is.
defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("peers");
</script>

<template>
  <SectionCard v-if="drawn" :title="t('settings.session_messages')" :hint="t('remote.sessions_on_this_machine_send')">
    <PeersSection :port="settingsProps.port" :hub="settingsProps.hub" />
  </SectionCard>
</template>
