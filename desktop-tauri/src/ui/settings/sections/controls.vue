<script setup lang="ts">
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import SectionCard from "../../SectionCard.vue";
import KeysSheet from "../../KeysSheet.vue";
import EscStopSetting from "../../EscStopSetting.vue";
import PageSleepSetting from "../../PageSleepSetting.vue";
import LiveGraceSection from "../../LiveGraceSection.vue";
import LiveStreamBudgetSection from "../../LiveStreamBudgetSection.vue";
import OpenSessionLimitSection from "../../OpenSessionLimitSection.vue";
import NavRailSection from "../../NavRailSection.vue";
import SessionQuickSection from "../../SessionQuickSection.vue";
import SessionTabsSetting from "../../SessionTabsSetting.vue";
import RunningMessageSetting from "../../RunningMessageSetting.vue";
import ComposerHintSection from "../../ComposerHintSection.vue";
import LookNavEntries from "../parts/look-naventries.vue";
import LookShortcut from "../parts/look-shortcut.vue";

// Nothing in this section belongs to a session: keys, tabs and idle behaviour
// are the window's, and the shell that owns them is the one above every pane.
defineProps<{ settingsProps: SettingsProps }>();
</script>

<template>
  <SectionCard :title="t('settings.keyboard_shortcuts')">
    <KeysSheet />
    <LookShortcut />
    <EscStopSetting />
  </SectionCard>

  <SectionCard :title="t('settings.tabs_and_navigation_card')">
    <SessionTabsSetting />
    <SessionQuickSection />
    <NavRailSection />
    <LookNavEntries />
    <LiveGraceSection />
  </SectionCard>

  <SectionCard :title="t('settings.while_running_card')">
    <RunningMessageSetting />
    <ComposerHintSection />
  </SectionCard>

  <SectionCard :title="t('settings.performance_card')">
    <OpenSessionLimitSection />
    <LiveStreamBudgetSection />
    <PageSleepSetting />
  </SectionCard>
</template>
