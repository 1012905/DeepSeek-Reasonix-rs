<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { LIVE_GRACE_CHOICES, liveGraceMinutes, onLiveGraceChange, setLiveGraceMinutes } from "../state/prefs";
import Group from "./Group.vue";

const LABEL: Record<number, string> = { 0: "settings.off", 10: "settings.n_10_minutes", 30: "settings.n_30_minutes", 60: "settings.n_1_hour", 240: "settings.n_4_hours" };

const minutes = useExternal(onLiveGraceChange, liveGraceMinutes);
</script>

<template>
  <Group id="live-grace" inline :title="t('settings.keep_finished_sessions_in_running')" :hint="t('settings.finished_sessions_stay_in_running')">
    <div class="seg" data-text role="group" :aria-label="t('settings.keep_finished_sessions_in_running')">
      <button v-for="m in LIVE_GRACE_CHOICES" :key="m" data-action="appearance.live-grace" :data-value="String(m)" :aria-pressed="minutes === m" @click="setLiveGraceMinutes(m)">
        {{ t(LABEL[m]) }}
      </button>
    </div>
  </Group>
</template>