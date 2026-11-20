<script setup lang="ts">
import { t } from "../i18n";
import { setRunningMessageIntent, useRunningMessageIntent } from "../state/prefs";
import Group from "./Group.vue";

const intent = useRunningMessageIntent();
</script>

<template>
  <Group id="running-message-intent" :title="t('settings.messages_sent_while_running')" :hint="t('settings.what_a_message_sent_while_running_does')">
    <div class="seg" data-text="true" role="radiogroup" :aria-label="t('settings.messages_sent_while_running')">
      <button role="radio" data-action="composer.running-message-intent" data-value="steer" :aria-checked="intent === 'steer'" @click="setRunningMessageIntent('steer')">{{ t("settings.steer_at_the_next_tool") }}</button>
      <button role="radio" data-action="composer.running-message-intent" data-value="followup" :aria-checked="intent === 'followup'" @click="setRunningMessageIntent('followup')">{{ t("settings.send_after_the_whole_run") }}</button>
    </div>
    <p class="hint">{{ t("settings.use_send_now_in_the") }}</p>
  </Group>
</template>