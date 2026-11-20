<script setup lang="ts">
import { t } from "../i18n";
import type { SettingEntry } from "./prefsnav";

// A page that holds only one or two settings does not need an index: the row
// under the title would repeat what is already on screen.
defineProps<{ items: readonly SettingEntry[]; onGo: (entry: SettingEntry) => void }>();
</script>

<template>
  <nav v-if="items.length >= 3" class="page-index" :aria-label="t('settings.on_this_page')">
    <span class="pi-label" aria-hidden="true">{{ t("settings.on_this_page") }}</span>
    <button
      v-for="entry in items"
      :key="entry.anchor"
      type="button"
      class="spill"
      data-action="settings.page-index"
      :data-target="entry.anchor"
      @click="onGo(entry)"
    >
      {{ t(entry.title) }}
    </button>
  </nav>
</template>
