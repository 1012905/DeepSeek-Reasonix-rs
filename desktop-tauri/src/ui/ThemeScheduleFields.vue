<script setup lang="ts">
import { t } from "../i18n";
import { onThemeScheduleChange, setThemeSchedule, themeSchedule } from "../state/themeSchedule";
import { useExternal } from "../state/external";

const FIELDS = [["start", "look.dark_mode_starts"], ["end", "look.light_mode_resumes"]] as const;

const schedule = useExternal(onThemeScheduleChange, themeSchedule);
const change = (field: "start" | "end", e: Event) => setThemeSchedule({ ...schedule.value, [field]: (e.target as HTMLInputElement).value });
</script>

<template>
  <p class="hint" id="theme-schedule-hint">{{ t("look.switches_daily_in_local_time") }}</p>
  <label v-for="[field, label] in FIELDS" :key="field" class="prow">
    <span class="tx">{{ t(label) }}</span>
    <input type="time" class="fontown" :value="schedule[field]" required :step="60"
      aria-describedby="theme-schedule-hint" data-action="appearance.schedule" :data-target="field"
      @change="change(field, $event)" />
  </label>
  <p v-if="schedule.start === schedule.end" class="hint">{{ t("look.equal_start_and_end_times") }}</p>
</template>