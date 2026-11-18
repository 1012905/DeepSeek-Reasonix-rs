<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type { AgentPort } from "../../../port/port";
import type { ObservabilityPatch, ObservabilitySettings } from "../../../port/boundary";
import Group from "../../Group.vue";
import Switch from "../../Switch.vue";

type Field = "endpoint" | "headerName" | "headerEnv" | "serviceName";

// Hands each turn to the observability stack the person already runs, as
// OpenTelemetry traces. Off until they switch it on and name a collector; what is
// sent is structure, timing, counts and names — never prompts, tool arguments,
// results or paths.
const props = defineProps<{ port: AgentPort }>();

const state = ref<ObservabilitySettings | null>(null);
const draft = ref<Record<Field, string>>({ endpoint: "", headerName: "", headerEnv: "", serviceName: "" });
const busy = ref(false);
const error = ref("");
const readOnly = ref(false);

const adopt = (s: ObservabilitySettings) => {
  state.value = s;
  draft.value = { endpoint: s.endpoint, headerName: s.headerName, headerEnv: s.headerEnv, serviceName: s.serviceName };
};

watch(() => props.port, (port) => {
  let live = true;
  port.observabilitySettings().then((s) => { if (live) adopt(s); }).catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

async function apply(patch: ObservabilityPatch): Promise<void> {
  busy.value = true;
  error.value = "";
  try {
    adopt(await props.port.saveObservabilitySettings(patch));
  } catch (e) {
    // A server that will not write its config says so by code; the form goes
    // read-only rather than offering a second save that will be refused too.
    if (e instanceof HttpError && e.reason?.code === "observability.editing_disabled") readOnly.value = true;
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// A field is committed when it is left, not on every keystroke: each save is a
// config write, and a half-typed collector address is not one.
function commit(field: Field): void {
  const held = state.value;
  if (!held || readOnly.value) return;
  const next = draft.value[field].trim();
  if (next !== held[field]) void apply({ [field]: next } as ObservabilityPatch);
}

const field = (key: Field) => (e: Event) => { draft.value = { ...draft.value, [key]: (e.target as HTMLInputElement).value }; };

// The address first, then what authenticates to it, then what names this build
// in the traces: the order a collector's own documentation is read in.
const FIELDS: [Field, string, string][] = [
  ["endpoint", "data.collector_address", "http://localhost:4318"],
  ["headerName", "data.auth_header_name", "Authorization"],
  ["headerEnv", "data.environment_variable_holding_the_credential", "OTEL_EXPORTER_AUTH"],
  ["serviceName", "data.service_name", "reasonix"],
];
</script>

<template>
  <!-- The block is drawn before its state arrives: a setting with no row on
       screen is one the search can find and not scroll to. -->
  <Group id="trace-export" :title="t('data.trace_export')" :hint="t('settings.trace_hint')">
    <div v-if="!state" class="empty">{{ t("common.loading") }}</div>
    <div v-else class="box">
      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("data.export_traces") }}</span>
          <span class="ds">
            {{ !state.enabled ? t("data.off_now_nothing_is_sent")
              : state.endpoint ? t("data.on_sessions_opened_from_now")
              : t("data.on_but_there_is_no") }}
          </span>
        </span>
        <Switch
          data-action="trace-export.enabled"
          :on="state.enabled"
          :busy="busy || readOnly"
          :label="t('data.export_traces')"
          :on-click="() => apply({ enabled: !state!.enabled })"
        />
      </div>
      <div v-for="[key, label, placeholder] in FIELDS" :key="key" class="bk-fields">
        <label class="full">
          <span>{{ t(label) }}</span>
          <input
            :id="`trace-${key}`"
            data-action-change="trace-export.field"
            :data-value="key"
            :aria-label="t(label)"
            :placeholder="placeholder"
            :value="draft[key]"
            :disabled="busy || readOnly"
            spellcheck="false"
            @input="field(key)"
            @blur="commit(key)"
          />
        </label>
      </div>
      <div class="peernote">
        <template v-if="state.headerEnv">
          {{ t(state.headerEnvSet ? "data.the_variable_is_set_its" : "data.the_variable_has_no_value") }}
        </template>
        <template v-else>{{ t("data.the_credential_is_never_in") }}</template>
      </div>
      <div class="peernote">{{ t("data.a_save_applies_to_sessions") }}<code>{{ state.path }}</code></div>
      <div v-if="readOnly" class="peernote" role="status">{{ t("common.these_settings_are_read_only") }}</div>
      <div v-if="error" class="why" role="alert">{{ error }}</div>
    </div>
  </Group>
</template>
