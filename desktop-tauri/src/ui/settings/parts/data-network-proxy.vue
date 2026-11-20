<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, say } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { NetworkProbe, NetworkSettings } from "../../../port/network";
import StudioIcon from "../../StudioIcon.vue";

// Three modes rather than six input boxes. Nobody's first question is whether
// they want http_proxy or https_proxy; it is "use what the system uses" or
// "here is mine".
const MODES: [string, string][] = [
  ["auto", "look.follow_the_system"],
  ["custom", "models.set_manually"],
  ["off", "models.direct"],
];

const STEP_LABEL: Record<string, string> = {
  proxy: "models.proxy",
  dns: "models.name_resolution",
  connect: "models.connection",
  tls: "models.tls_handshake",
  auth: "models.credentials",
};

const TYPES = ["http", "https", "socks5", "socks5h"];

// Only the fields the form owns: the rest of the answer describes the saved row
// and comparing it would report a change nobody made.
const EDITABLE = ["mode", "type", "server", "port", "username", "noProxy"] as const;

const props = defineProps<{ port: AgentPort; host?: string }>();

const saved = ref<NetworkSettings | null>(null);
const draft = ref<NetworkSettings | null>(null);
const password = ref("");
const clearPassword = ref(false);
const probes = ref<NetworkProbe[] | null>(null);
const busy = ref<"" | "save" | "test">("");
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  saved.value = null;
  draft.value = null;
  probes.value = null;
  error.value = "";
  port.network()
    .then((net) => { if (live) { saved.value = net; draft.value = net; } })
    .catch((e) => { if (live) error.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

const changed = computed(() => {
  const held = saved.value;
  const now = draft.value;
  if (!held || !now) return false;
  return EDITABLE.some((key) => now[key] !== held[key]) || password.value !== "" || clearPassword.value;
});

function patch(p: Partial<NetworkSettings>): void {
  const now = draft.value;
  if (!now || busy.value === "save") return;
  draft.value = { ...now, ...p };
}

async function save(): Promise<void> {
  const now = draft.value;
  if (!now || busy.value || !changed.value) return;
  busy.value = "save";
  error.value = "";
  try {
    const result = await props.port.saveNetwork(now, password.value, clearPassword.value);
    saved.value = result;
    draft.value = result;
    password.value = "";
    clearPassword.value = false;
    // The old probes described the old settings; keeping them on screen would
    // claim a path that has not been tested since.
    probes.value = null;
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = "";
  }
}

async function diagnose(): Promise<void> {
  if (busy.value) return;
  busy.value = "test";
  error.value = "";
  try {
    probes.value = await props.port.diagnoseNetwork();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = "";
  }
}

const digits = (e: Event) => patch({ port: Number((e.target as HTMLInputElement).value.replace(/\D/g, "")) || 0 });
const text = (key: keyof NetworkSettings) => (e: Event) => patch({ [key]: (e.target as HTMLInputElement).value } as Partial<NetworkSettings>);
// Only the automatic mode needs a sentence of its own: the other two are what
// the form below already shows.
const modeSaid = computed(() => (draft.value?.mode === "auto" ? t("models.uses_whatever_proxy_the_system") : ""));
</script>

<template>
  <div v-if="error && !draft" class="empty" role="alert">{{ error }}</div>
  <div v-else-if="!draft || !saved" class="empty" role="status">{{ t("models.loading_network_settings") }}</div>
  <div v-else class="net">
    <p v-if="host" class="note">{{ t("models.these_are_the_network_settings", { host }) }}</p>
    <div class="seg" data-text role="radiogroup" :aria-label="t('models.proxy_mode')">
      <button
        v-for="[id, label] in MODES"
        :key="id"
        role="radio"
        data-action="network.mode"
        :data-value="id"
        :aria-checked="draft.mode === id"
        :disabled="busy === 'save'"
        @click="patch({ mode: id })"
      >{{ t(label) }}</button>
    </div>
    <p v-if="modeSaid" class="note">{{ modeSaid }}</p>

    <!-- The values sit in `code` so the mono face is on the machine string and
         not on the row: these are addresses, and a row that also carries a label
         must not read as one. -->
    <div class="kv">
      <span class="k">{{ t("common.in_effect") }}</span>
      <span class="v"><code>{{ saved.effective || "—" }}</code></span>
    </div>
    <div v-if="saved.endpoint" class="kv">
      <span class="k">{{ t("models.test_target") }}</span>
      <span class="v"><code>{{ saved.endpoint }}</code></span>
    </div>
    <div v-if="saved.direct?.length" class="kv">
      <span class="k">{{ t("models.always_connect_directly") }}</span>
      <span class="v"><code>{{ (saved.direct ?? []).join(", ") }}</code></span>
    </div>

    <div v-if="draft.mode === 'custom'" class="fields">
      <label>
        <span>{{ t("models.protocol") }}</span>
        <select :aria-label="t('models.protocol')" data-action="network.type" :value="draft.type || 'http'" :disabled="busy === 'save'" @change="patch({ type: ($event.target as HTMLSelectElement).value })">
          <option v-for="k in TYPES" :key="k" :value="k">{{ k }}</option>
        </select>
      </label>
      <label class="grow">
        <span>{{ t("models.server") }}</span>
        <input :aria-label="t('models.server')" data-action="network.server" :value="draft.server ?? ''" :disabled="busy === 'save'" placeholder="proxy.corp" @input="text('server')" />
      </label>
      <label>
        <span>{{ t("common.port") }}</span>
        <input class="port" :aria-label="t('common.port')" data-action="network.port" inputmode="numeric" :disabled="busy === 'save'" :value="draft.port ? String(draft.port) : ''" placeholder="8080" @input="digits" />
      </label>
      <label class="grow">
        <span>{{ t("models.username_optional") }}</span>
        <input :aria-label="t('models.username_optional')" data-action="network.username" :value="draft.username ?? ''" :disabled="busy === 'save'" @input="text('username')" />
      </label>
      <label class="grow">
        <span>{{ t("models.password_optional") }}</span>
        <input
          type="password"
          :aria-label="t('models.password_optional')"
          data-action="network.password"
          :disabled="busy === 'save'"
          :value="password"
          :placeholder="t(draft.hasPassword ? 'models.saved_leave_blank_to_keep' : 'models.you_can_write_proxy_password')"
          @input="password = ($event.target as HTMLInputElement).value; clearPassword = false"
        />
      </label>
      <label v-if="draft.hasPassword && !password" class="chk">
        <input type="checkbox" data-action="network.clear-password" :checked="clearPassword" :disabled="busy === 'save'" @change="clearPassword = ($event.target as HTMLInputElement).checked" />
        <span>{{ t("models.delete_the_saved_password") }}</span>
      </label>
      <label class="grow full">
        <span>{{ t("models.reach_these_addresses_directly") }}</span>
        <input :aria-label="t('models.reach_these_addresses_directly')" data-action="network.no-proxy" :value="draft.noProxy ?? ''" :disabled="busy === 'save'" placeholder="localhost, 127.0.0.1, *.internal" @input="text('noProxy')" />
      </label>
    </div>

    <div class="acts">
      <button type="button" class="act" data-action="network.diagnose" :disabled="!!busy" @click="diagnose">
        {{ t(busy === "test" ? "models.testing_2" : "models.test_saved_settings") }}
      </button>
      <button type="button" class="act" data-primary data-action="network.save" :disabled="!!busy || !changed" @click="save">
        {{ t(busy === "save" ? "common.saving" : "common.save") }}
      </button>
    </div>
    <p v-if="changed" class="note" role="status">{{ t("models.there_are_unsaved_changes_connection") }}</p>

    <div v-if="probes" class="probes" role="region" :aria-label="t('models.test_results_for_saved_settings')">
      <div v-for="(p, i) in probes" :key="i" class="probe" :data-ok="p.ok ? '' : undefined">
        <i class="mk"><StudioIcon :name="p.ok ? 'check' : 'close'" /></i>
        <span class="lb">{{ STEP_LABEL[p.step] ? t(STEP_LABEL[p.step]) : p.step }}</span>
        <span class="dt">{{ say(p.code ? { code: p.code, params: p.params } : null, p.detail) }}</span>
        <span class="ms">{{ p.durationMs ? `${p.durationMs}ms` : "" }}</span>
        <span v-if="p.advice" class="adv">{{ say(p.adviceCode ? { code: p.adviceCode, params: p.params } : null, p.advice) }}</span>
      </div>
    </div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
