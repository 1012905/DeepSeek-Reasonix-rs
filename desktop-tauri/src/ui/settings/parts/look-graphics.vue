<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../../i18n";
import { host, type GraphicsInfo } from "../../../port/host";
import { onSoftwareRenderingChange, setSoftwareRendering, wantsSoftwareRendering } from "../../../state/prefs";
import { useExternal } from "../../../state/external";
import Group from "../../Group.vue";

// Which way the window is drawn this launch, and the one switch the shell can
// honour: Chromium fixes its GPU use before the app is ready, so a change is
// saved now and applies on the next launch. Absent in a browser tab, which has
// no GPU policy of its own — the switch still writes the choice the shell reads.
const report = ref<GraphicsInfo | null>(null);
const software = useExternal(onSoftwareRenderingChange, wantsSoftwareRendering);

onMounted(() => {
  host().graphics().then((g) => { report.value = g; }).catch(() => { report.value = null; });
});

const modeSaid = computed(() => {
  const g = report.value;
  if (!g) return t("look.no_graphics_status_reported_yet");
  if (g.launchedOff) {
    return `${t("look.software_rendering")} · ${t(g.savedOff === false ? "look.hardware_acceleration_was_switched_off" : "look.hardware_acceleration_is_switched_off")}`;
  }
  return g.compositing ? t("look.graphics_compositing_status_value", { value: g.compositing }) : t("look.no_graphics_status_reported_yet");
});

// What is saved and what this launch runs with are two facts, and the row says
// both: a saved choice that has not taken effect yet would otherwise look broken.
const pending = computed(() => !!report.value && software.value !== (report.value.savedOff ?? report.value.launchedOff));
</script>

<template>
  <Group id="graphics" :title="t('look.graphics')" :hint="t('look.if_the_window_keeps_your')">
    <div class="seg" data-text role="group" :aria-label="t('look.graphics')">
      <button data-action="appearance.graphics" data-value="auto" :aria-pressed="!software" @click="setSoftwareRendering(false)">
        {{ t("look.system_default") }}
      </button>
      <button data-action="appearance.graphics" data-value="software" :aria-pressed="software" @click="setSoftwareRendering(true)">
        {{ t("look.software_only") }}
      </button>
    </div>
    <p class="note" :data-graphics="report ? (report.launchedOff ? 'software' : report.compositing ? 'reported' : 'unreported') : 'unreported'">
      {{ t("look.this_launch_uses_mode", { mode: modeSaid }) }}
    </p>
    <p v-if="pending" class="note" role="status">{{ t("look.saved_quit_and_reopen_studio") }}</p>
  </Group>
</template>
