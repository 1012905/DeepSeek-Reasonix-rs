<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, PresentFilesSettings } from "../../../port/port";
import Switch from "../../Switch.vue";
import StudioIcon from "../../StudioIcon.vue";

// Whether the model may hand files back as clickable cards, which is the
// kernel's [tools] present_files. Read per port, so a brokered pane's answer is
// the one shown; the switch waits on the read rather than guessing a default,
// because the two possible defaults are opposite answers on screen.
const props = defineProps<{ port: AgentPort }>();

const state = ref<PresentFilesSettings | null>(null);
const busy = ref(false);
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  state.value = null;
  error.value = "";
  port.presentFiles()
    .then((value) => { if (live) state.value = value; })
    .catch((e) => { if (live) error.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

const flip = () => {
  const now = state.value;
  if (!now || busy.value) return;
  busy.value = true;
  error.value = "";
  props.port
    .savePresentFiles(!now.enabled)
    .then((value) => { state.value = value; })
    .catch((e) => { error.value = reason(e); })
    .finally(() => { busy.value = false; });
};
</script>

<template>
  <Switch
    v-if="state"
    data-action="present-files.enabled"
    :on="state.enabled"
    :busy="busy"
    :label="t('workbench.enable_ai_file_delivery_cards')"
    :on-click="flip"
  />
  <div v-else class="empty">
    <StudioIcon name="loading" />
    {{ t(error ? "workbench.could_not_read_file_delivery" : "workbench.loading") }}
  </div>
  <div v-if="error" class="why" role="alert">{{ error }}</div>
</template>
