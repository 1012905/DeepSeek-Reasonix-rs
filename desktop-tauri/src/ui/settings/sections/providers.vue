<script setup lang="ts">
import { computed } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import SectionCard from "../../SectionCard.vue";
import CoreProviders from "../parts/core-providers.vue";

// The services page on its own, for a link that names it directly. It draws the
// same two blocks the model page draws — an account is a model source, and the
// sheet files providers under model — so both read one implementation rather
// than two that can drift.
const props = defineProps<{ settingsProps: SettingsProps }>();
const p = computed(() => props.settingsProps);
</script>

<template>
  <SectionCard :title="t('models.services')">
    <CoreProviders :port="p.port" :status="p.status" :at="p.at" :on-changed="p.onChanged" />
  </SectionCard>
</template>
