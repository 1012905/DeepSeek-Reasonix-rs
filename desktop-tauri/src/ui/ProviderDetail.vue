<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import type { ProviderCheck } from "../port/port";
import EditConn from "./EditConn.vue";
import type { Account, Port } from "./Providers.vue";
import { KIND_LABEL } from "./vendors";
import { PROVIDER_EDIT_DISABLED, reason } from "../i18n/kernel";
import { HttpError } from "../port/http_error";
import { checkFailure } from "./provider_check";
import StudioIcon from "./StudioIcon.vue";

// The selected account, edited in place. The protocol is a switch on it rather
// than a fact on a row, because both entries are the same key at the same host;
// testing the connection is what turns "which protocol did we record" back into
// a finding.
const props = defineProps<{
  a: Account;
  port: Port;
  busy: string;
  setBusy: (b: string) => void;
  kind: string;
  onProtocol: (kind: string) => void;
  onRemove: (name: string) => void;
  removeFailed: string;
  onRename: () => void;
  onEdited: () => void | Promise<void>;
  onChecked: () => void;
  onDirty: (dirty: boolean) => void;
  declare?: string;
}>();

const found = ref<ProviderCheck | null>(null);
// A refusal is not a failed probe. The kernel withholds these routes from a
// server reachable over the network, because adding a source writes a key
// into the credential store of the machine running the kernel.
const refused = ref("");
const unanswered = ref("");
// Revert and save both hand the form a fresh start from what is stored, so
// the form remounts only once the list holds what the kernel has.
const revision = ref(0);
const saved = ref("");
const entry = computed(() => props.a.byKind[props.kind] ?? props.a.byKind[props.a.kinds[0]]);
const checking = computed(() => props.busy === `check:${entry.value.name}`);
const inUse = computed(() => props.a.kinds.some((k) => props.a.byKind[k].inUse));
const kindLabel = (kind?: string) => t(KIND_LABEL[kind ?? ""] ?? kind ?? "");
const entrySub = computed(() => `${kindLabel(entry.value.kind)} · ${props.a.host}`);
const unlisted = computed(() => (found.value?.models ?? []).filter((m) => !entry.value.models.includes(m)));

const check = async () => {
  props.setBusy(`check:${entry.value.name}`);
  found.value = null;
  unanswered.value = "";
  refused.value = "";
  try {
    found.value = await props.port.checkProvider(entry.value.name);
    props.onChecked();
  } catch (e) {
    // Read off the code the kernel sent, never the status: 403 is also what a
    // gateway in front of it answers, and that is a different thing to do next.
    if (e instanceof HttpError && e.reason?.code === PROVIDER_EDIT_DISABLED) refused.value = reason(e);
    else unanswered.value = reason(e);
  } finally {
    props.setBusy("");
  }
};

const foundTitle = computed(() => {
  const f = found.value;
  if (!f) return "";
  return f.ok
    ? `${t("common.connected")} · ${kindLabel(f.kind)} · ${t("models.n_models", { n: f.models?.length ?? 0 })}`
    : t("models.unreachable");
});

// What the endpoint answered with that this connection does not list. The
// vendor adds models to endpoints we already know; a stored list never finds
// out on its own, and the probe is the one place that already has the answer.
const foundWhy = computed(() => {
  const f = found.value;
  if (!f) return "";
  if (!f.ok) return checkFailure(f);
  const parts: string[] = [];
  if (f.matches === false) {
    parts.push(t("models.recorded_as_had_but_it", { had: kindLabel(entry.value.kind), got: kindLabel(f.kind) }));
  } else {
    parts.push(t("models.the_key_works_and_the"));
  }
  if (f.baseUrl) parts.push(t("models.the_model_list_only_answers", { url: f.baseUrl }));
  if (f.noProxy) parts.push(t("models.the_proxy_cannot_reach_it"));
  if (unlisted.value.length > 0) {
    parts.push(t("models.this_endpoint_has_n_more", {
      n: unlisted.value.length,
      names: unlisted.value.slice(0, 3).join("、") + (unlisted.value.length > 3 ? "…" : ""),
    }));
  }
  return parts.join(" ");
});

const doneEditing = async () => {
  await props.onEdited();
  saved.value = entry.value.name;
  revision.value += 1;
};

const revert = () => {
  saved.value = "";
  revision.value += 1;
};
</script>

<template>
  <section class="pdetail" :aria-label="a.label">
    <header class="pdetail-hd">
      <span class="tx">
        <span class="pdetail-name">
          <h3>{{ a.label }}</h3>
          <button class="pdetail-rename" data-action="provider.rename-start" :data-target="a.key" :disabled="busy !== ''"
            :aria-label="t('models.rename_name_f2', { name: a.label })" :title="t('models.rename_name_f2', { name: a.label })"
            @click="onRename"><StudioIcon name="edit" /></button>
        </span>
        <small :title="entrySub">{{ entrySub }}</small>
      </span>
      <i v-if="inUse" class="pdetail-use">{{ t("models.in_use") }}</i>
      <button class="act" data-action="provider.probe" @click="check" :disabled="busy !== ''">
        {{ t(checking ? "models.testing_2" : "common.test_it") }}
      </button>
      <button class="act" data-danger data-action="provider.remove" :data-target="entry.name" @click="onRemove(entry.name)" :disabled="busy !== ''">
        {{ t("common.delete") }}
      </button>
    </header>
    <div v-if="removeFailed" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("common.that_did_not_go_through") }}</span>
      <span class="why">{{ removeFailed }}</span>
    </div>
    <div v-if="refused" class="find" data-lvl="warn" role="status">
      <span class="t">{{ refused }}</span>
      <span class="why">{{ t("models.model_sources_are_configured_on") }}</span>
    </div>
    <div v-if="unanswered" class="find" data-lvl="warn" role="status">
      <span class="t">{{ t("models.unreachable") }}</span>
      <span class="why">{{ unanswered }}</span>
    </div>
    <div v-if="found" class="find" :data-lvl="found.ok ? 'ok' : 'warn'" role="status">
      <span class="t">{{ foundTitle }}</span>
      <span class="why">{{ foundWhy }}</span>
    </div>
    <div v-if="a.kinds.length > 1" class="pdetail-options">
      <div class="vway">
        <span class="lb">{{ t("common.protocol") }}</span>
        <div class="seg" role="group" :aria-label="t('models.protocol_for_name', { name: a.label })">
          <button v-for="k in a.kinds" :key="k" data-action="provider.protocol" :data-target="entry.name" :data-value="k" :aria-pressed="k === kind" :disabled="busy !== ''" @click="onProtocol(k)">
            {{ t(KIND_LABEL[k] ?? k) }}
            <!-- A door that carries a capability the other lacks has to say
                 so on itself: switching is otherwise a silent downgrade. -->
            <i v-if="a.byKind[k].canWebSearch" class="perk">{{ t("models.web_search") }}</i>
          </button>
        </div>
        <span class="why">
          {{ t(a.kinds.some((k) => a.byKind[k].canWebSearch) && !entry.canWebSearch ? "models.two_ways_in_to_the_2" : "models.two_ways_in_to_the") }}
        </span>
      </div>
    </div>
    <EditConn
      :key="`${entry.name}:${revision}`"
      :entry="entry"
      :label="a.label"
      :wire-fixed="a.kinds.length > 1"
      :initial-check="found?.ok ? found : undefined"
      :port="port"
      :busy="busy"
      :set-busy="setBusy"
      :declare="!!declare && entry.name === declare"
      :just-saved="saved === entry.name"
      :on-dirty="onDirty"
      :on-done="doneEditing"
      :on-revert="revert"
      :on-saved="onEdited"
    />
  </section>
</template>
