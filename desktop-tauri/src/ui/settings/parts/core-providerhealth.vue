<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type { AgentPort } from "../../../port/port";
import SettingRow from "../../SettingRow.vue";
import Switch from "../../Switch.vue";

// Whether opening the model list also asks each endpoint what it is. Two kernels
// answer this: one that predates the route answers 404, and one reachable over
// the network withholds it. Saying which of the two this is beats a switch that
// does nothing, so both are told apart rather than collapsed into "failed".
const props = defineProps<{
  port: AgentPort;
  onChanged: () => void;
  onFailed: (why: string) => void;
}>();

const enabled = ref<boolean | null>(null);
const busy = ref(false);
const unavailable = ref<"unsupported" | "failed" | null>(null);

// Every read and write carries the generation it started in: a port swapped
// while a save is in flight must not let the old answer land on the new switch.
let generation = 0;

const load = () => {
  const version = ++generation;
  enabled.value = null;
  busy.value = false;
  unavailable.value = null;
  props.port
    .providerHealthSettings()
    .then((value) => {
      if (version === generation) enabled.value = value.enabled;
    })
    .catch((e) => {
      if (version !== generation) return;
      unavailable.value = e instanceof HttpError && [404, 405, 501].includes(e.status) ? "unsupported" : "failed";
    });
};

const flip = async () => {
  if (enabled.value === null) return;
  const version = generation;
  busy.value = true;
  try {
    const value = await props.port.saveProviderHealthSettings(!enabled.value);
    if (version === generation) {
      enabled.value = value.enabled;
      props.onChanged();
    }
  } catch (e) {
    if (version === generation) props.onFailed(reason(e));
  } finally {
    if (version === generation) busy.value = false;
  }
};

onMounted(load);
watch(() => props.port, load);
</script>

<template>
  <SettingRow
    id="provider-health"
    :title="t('models.automatically_check_connections_and_balances')"
    :desc="t('models.check_while_the_list_opens')"
  >
    <Switch
      data-action="provider.health-setting"
      :on="enabled === true"
      :busy="busy || enabled === null"
      :label="t('models.automatically_check_connections_and_balances')"
      :on-click="flip"
    />
  </SettingRow>
  <p v-if="unavailable" class="note" role="status">
    {{ t(unavailable === "unsupported" ? "models.this_server_does_not_support" : "models.cannot_read_check_settings_check") }}
  </p>
</template>
