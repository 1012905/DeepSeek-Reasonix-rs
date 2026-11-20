<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, BrowserToolsSettings } from "../../../port/port";
import Switch from "../../Switch.vue";

// One switch over whether the browser tools are bound at all. The row reads the
// user file; a project file that sets the same key outranks it, and then what
// this workspace runs with is said beside the switch.
const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const state = ref<BrowserToolsSettings | null>(null);
const busy = ref(false);
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  state.value = null;
  error.value = "";
  port.browserTools()
    .then((value) => { if (live) state.value = value; })
    .catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

const flip = async () => {
  const now = state.value;
  if (!now || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    state.value = await props.port.saveBrowserTools(!now.enabled);
    props.onChanged();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div v-if="!state" class="empty">{{ t("workbench.could_not_read_the_built") }}</div>
  <div v-else>
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("common.built_in_browser") }}</span>
        <span class="ds">{{ t("workbench.let_it_open_pages_in_the_browser") }}</span>
      </span>
      <Switch
        data-action="browser-tools.enabled"
        :on="state.enabled"
        :busy="busy"
        :label="t('workbench.enable_the_built_in_browser')"
        :on-click="() => void flip()"
      />
    </div>
    <p v-if="state.effective !== state.enabled" class="note">
      {{ t("common.actually_in_force") }} ·
      {{ t(state.effective ? "workbench.this_projects_config_file_turns_2" : "workbench.this_projects_config_file_turns") }}
    </p>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
