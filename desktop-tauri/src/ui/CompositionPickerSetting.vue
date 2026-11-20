<script setup lang="ts">
import { h, ref } from "vue";
import { t } from "../i18n";
import { setShowsCompositionPicker, useShowsCompositionPicker } from "../state/compositionPicker";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const enabled = useShowsCompositionPicker();
const error = ref(false);
const action = () => h(Switch, {
  label: t("persona.show_mode_picker"),
  "data-action": "agent-preset.show-picker",
  on: enabled.value,
  onClick: () => (error.value = !setShowsCompositionPicker(!enabled.value)),
});
</script>

<template>
  <Group id="composition-picker" :title="t('persona.show_mode_picker')" :hint="t('persona.off_by_default_enable_to')" :action="action()">
    <p v-if="error" role="alert">{{ t("persona.could_not_save_the_mode") }}</p>
  </Group>
</template>