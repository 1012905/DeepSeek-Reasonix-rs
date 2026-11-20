<script setup lang="ts">
import { computed } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import SectionCard from "../../SectionCard.vue";
import Group from "../../Group.vue";
import Compaction from "../../Compaction.vue";
import CoreModelUsage from "../parts/core-modelusage.vue";
import CoreProviders from "../parts/core-providers.vue";

// Which service does which job, and what happens when a conversation outgrows
// its window. The services blocks sit here too: the sheet files providers under
// model (FOLDS_INTO), so this page is the one a "providers:providers" link lands
// on and it owes those two anchors as much as its own.
const props = defineProps<{ settingsProps: SettingsProps }>();
const p = computed(() => props.settingsProps);
const home = computed(() => p.value.networkPort ?? p.value.port);
</script>

<template>
  <SectionCard :title="t('models.services')">
    <CoreProviders :port="p.port" :status="p.status" :at="p.at" :on-changed="p.onChanged" />
  </SectionCard>

  <SectionCard :title="t('models.uses_and_context')">
    <Group
      id="model"
      :title="t('settings.models_by_use')"
      :hint="t('models.which_model_for_each_use')"
    >
      <CoreModelUsage
        :port="p.port"
        :home="home"
        :model-ref="p.status?.modelRef"
        :effort="p.status?.effort"
        :on-changed="p.onChanged"
      />
    </Group>

    <!-- The window share, not a threshold: this row reports where the session
         will fold and offers nothing to type, because the number is derived
         from the model's own declared window. -->
    <Group id="context" :title="t('settings.context_maintenance')">
      <Compaction :port="p.port" />
    </Group>
  </SectionCard>
</template>
