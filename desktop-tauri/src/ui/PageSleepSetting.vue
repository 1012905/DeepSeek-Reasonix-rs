<script setup lang="ts">
import { t } from "../i18n";
import { setPageSleep, usePageSleep } from "../state/pagesleep";
import Group from "./Group.vue";

const mode = usePageSleep();
</script>

<template>
  <Group id="page-sleep" :inline="true" :title="t('settings.idle_page_sleep')" :hint="t('settings.how_long_before_a_page_sleeps')">
    <div class="seg" data-text="true" role="radiogroup" :aria-label="t('settings.idle_page_sleep')">
      <button role="radio" data-action="appearance.page-sleep" data-value="off" :aria-checked="mode === 'off'" @click="setPageSleep('off')">{{ t("settings.keep_open") }}</button>
      <button role="radio" data-action="appearance.page-sleep" data-value="on" :aria-checked="mode === 'on'" @click="setPageSleep('on')">{{ t("settings.on_after_15_minutes") }}</button>
    </div>
  </Group>
</template>