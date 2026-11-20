<script setup lang="ts">
import { t } from "../i18n";
import { setEscStop, useEscStop } from "../state/escstop";
import Group from "./Group.vue";

const mode = useEscStop();
</script>

<template>
  <Group id="esc-stop" :title="t('safety.esc_stops_a_task')" :hint="t('safety.what_escape_does_while_a')">
    <div class="seg" data-text role="radiogroup" :aria-label="t('safety.esc_stops_a_task')">
      <button role="radio" data-action="appearance.esc-stop" data-value="confirm" :aria-checked="mode === 'confirm'" @click="setEscStop('confirm')">{{ t("safety.press_esc_twice_to_stop") }}</button>
      <button role="radio" data-action="appearance.esc-stop" data-value="now" :aria-checked="mode === 'now'" @click="setEscStop('now')">{{ t("safety.press_esc_once_to_stop") }}</button>
    </div>
  </Group>
</template>