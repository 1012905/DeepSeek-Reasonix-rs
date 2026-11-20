<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import { drawsHere } from "./nesting";
import UsagePanel from "../parts/data-usage.vue";
import UploadUsage from "../parts/data-upload-usage.vue";
import DisplayCurrency from "../parts/data-display-currency.vue";

// What this machine has spent: tokens on one side, money on the other, and the
// currency the money is shown in. Reading the report is keyed on the currency
// choice — its amounts are read once, so a new currency has to bring a new read
// rather than relabel the old figures.
//
// Two pages draw these blocks: this one when `at` names "usage" and storage,
// which usage folds into. The column holds one of them at a time; ./nesting is
// what keeps it one if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("usage");

const home = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
const rev = ref(0);
</script>

<template>
  <SectionCard v-if="drawn" :title="t('data.memory_and_usage')" :hint="t('settings.usage_hint')">
    <Group id="usage" :title="t('usage.usage_cost')" :hint="t('settings.usage_hint')">
      <UsagePanel :key="rev" :port="home" />
    </Group>

    <Group id="upload-usage" :title="t('usage.upload_volume')" :hint="t('settings.size_of_requests_sent')">
      <UploadUsage :port="home" />
    </Group>

    <DisplayCurrency :port="home" :on-changed="() => { rev++; settingsProps.onChanged(); }" />
  </SectionCard>
</template>
