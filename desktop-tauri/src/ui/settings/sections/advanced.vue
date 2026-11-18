<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import ChangedSettings from "../parts/data-changed-settings.vue";

// What used to be a config-file-only knob and has no screen yet. The list is
// declared, not derived: it names what is known to be missing, so the page says
// where those things live now instead of drawing an empty section for them.
const ELSEWHERE: string[] = [];

const props = defineProps<{ settingsProps: SettingsProps }>();

const home = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
const previousApp = t("settings.previous_desktop_app");
</script>

<template>
  <SectionCard :title="t('settings.advanced')">
    <Group id="elsewhere" :title="t('settings.not_in_this_version_yet')" :hint="t('settings.the_items_below_have_no')">
      <div v-for="item in ELSEWHERE" :key="item" class="lrow">
        <span class="tx"><span class="lb">{{ item }}</span></span>
        <span class="sc">{{ previousApp }}</span>
      </div>
      <!-- The list of configuration this machine has changed is not one of those:
           it reads what is already on disk and reports it. -->
      <ChangedSettings :port="home" />
    </Group>
  </SectionCard>
</template>
