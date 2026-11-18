<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort, PeersSettings, PeersSettingsEdit } from "../port/port";
import type { PeersSettingsStore } from "./usePeersSettings";
import Switch from "./Switch.vue";

/** The switches behind session messages, which used to be hand-written config.
 *  Saved values only reach the model at the next turn boundary — the tool table
 *  changes with them — so a saved edit says exactly that until the kernel does. */
const props = defineProps<{ port: AgentPort; settings: PeersSettingsStore; onSaved?: (next: PeersSettings) => void }>();

const state = computed(() => props.settings.state.value);
const loadError = computed(() => props.settings.loadError.value);
const publish = (next: PeersSettings) => props.settings.publish(next);

const busy = ref(false);
const error = ref("");

const save = async (edit: PeersSettingsEdit) => {
  busy.value = true;
  error.value = "";
  try {
    const next = await props.port.savePeersSettings(edit);
    publish(next);
    props.onSaved?.(next);
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const toggleEnabled = () => void save({ enabled: !state.value?.enabled });
const toggleDispatch = () => void save({ dispatch: !state.value?.dispatch });
const toggleNested = () => void save({ open_nested: !state.value?.open_nested });

const shownError = computed(() => error.value || loadError.value);
const off = computed(() => busy.value || !state.value?.available);
</script>

<template>
  <div v-if="!state && shownError" class="why" role="alert">{{ shownError }}</div>
  <div v-else-if="state" class="box">
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("remote.turn_on_session_messages") }}</span>
        <span class="ds">{{ t("remote.sessions_on_this_machine_send") }}</span>
      </span>
      <Switch data-action="peers.enable" :on="state.enabled" :busy="off" :label="t('remote.turn_on_session_messages')" :on-click="toggleEnabled" />
    </div>
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("remote.let_the_model_hand_a") }}</span>
        <span class="ds">{{ state.enabled ? t("remote.gives_the_model_a_tool") : t("remote.turn_on_session_messages_first_2") }}</span>
      </span>
      <Switch data-action="peers.dispatch" :on="state.dispatch" :busy="off || !state.enabled" :label="t('remote.let_the_model_hand_a')" :on-click="toggleDispatch" />
    </div>
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("remote.open_nested_folders_without_asking") }}</span>
        <span class="ds">{{ t("remote.open_nested_folders_without_asking_ds") }}</span>
      </span>
      <Switch data-action="peers.open-nested" :on="state.open_nested" :busy="off || !state.enabled || !state.dispatch" :label="t('remote.open_nested_folders_without_asking')" :on-click="toggleNested" />
    </div>
    <div v-if="!state.available" class="peernote" role="status">{{ t("remote.not_supported_in_this_session") }}</div>
    <div v-if="state.pending" class="peernote" role="status">{{ t("remote.saved_takes_effect_from_the") }}</div>
    <div class="peernote">{{ t("remote.these_switches_are_kept_in") }}<code>{{ state.path }}</code></div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
