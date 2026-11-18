<script setup lang="ts">
import { onMounted } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import { FEEDBACK_RETIRED } from "../../../feedbackchannel";
import { t } from "../../../i18n";
import { loadChangedFilesPref } from "../../../state/changedfiles";
import { loadFoldWorkPref } from "../../../state/foldwork";
import SectionCard from "../../SectionCard.vue";
import Group from "../../Group.vue";
import ChangedFilesSetting from "../../ChangedFilesSetting.vue";
import FoldWorkSetting from "../../FoldWorkSetting.vue";
import FeedbackLevelNoticeSetting from "../../FeedbackLevelNoticeSetting.vue";
import NewAnswerSection from "../../NewAnswerSection.vue";
import OfficePreviewSection from "../../OfficePreviewSection.vue";
import PlanProgressSection from "../../PlanProgressSection.vue";
import LookFolding from "../parts/look-folding.vue";
import LookPresentFiles from "../parts/look-present-files.vue";

const props = defineProps<{ settingsProps: SettingsProps }>();

// The two kernel-held switches are read here as well as in the transcript: a
// pane adopts them on mount, but settings may be the first screen this window
// shows, and an unread switch would draw the default as if it were the choice.
onMounted(() => {
  void loadChangedFilesPref(props.settingsProps.port);
  void loadFoldWorkPref(props.settingsProps.port);
});
</script>

<template>
  <SectionCard :title="t('settings.process_card')">
    <FoldWorkSetting :port="settingsProps.port" />
    <LookFolding />
    <PlanProgressSection />
  </SectionCard>

  <SectionCard :title="t('settings.files_and_previews_card')">
    <ChangedFilesSetting :port="settingsProps.port" />
    <Group
      id="present-files"
      inline
      :title="t('settings.ai_file_delivery_cards')"
      :hint="t('settings.generated_files_as_clickable_cards')"
    >
      <LookPresentFiles :port="settingsProps.port" />
    </Group>
    <OfficePreviewSection />
  </SectionCard>

  <SectionCard :title="t('settings.notices_card')">
    <NewAnswerSection />
    <!-- Retired with the feedback channel: the row stays declared in the
         settings table so a saved config still names a real anchor. -->
    <FeedbackLevelNoticeSetting v-if="!FEEDBACK_RETIRED" :port="settingsProps.port" />
  </SectionCard>
</template>
