<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../i18n";
import type { Protocol, ProviderEntry, ProviderPreset, ProviderProbe } from "../port/port";
import type { Port } from "./Providers.vue";
import { reason } from "../i18n/kernel";
import { HttpError } from "../port/http_error";
import { KIND_LABEL, hostOf, nameFrom, sourceNameUsable, vendorLabel } from "./vendors";
import ModelChoice, { checkedFact, clearModelCheckFacts, type ModelFact } from "./ModelChoice.vue";
import EffortShape from "./EffortShape.vue";
import { THINKING, parseExtraBody, parseHeaders } from "./provider_compat";
import { useModelBatch } from "./useModelBatch";
import PresetGallery from "./PresetGallery.vue";
import PresetInstall from "./PresetInstall.vue";
import StudioIcon from "./StudioIcon.vue";

// Detection is assistance, not authority. Compatible relays often expose one
// model-list shape while expecting another request protocol, so every durable
// value remains editable and a failed probe never blocks manual setup.
const NAME_CODES = new Set(["provider.name_required", "provider.name_invalid", "provider.name_taken"]);

const HEADERS_PLACEHOLDER = "HTTP-Referer: https://example.com\nX-Title: Reasonix";
const EXTRA_PLACEHOLDER = '{\n  "temperature": 0.7\n}';

const props = defineProps<{
  port: Port;
  taken: string[];
  known: ProviderEntry[];
  onDone: () => void;
  onCancel: () => void;
}>();

const baseUrl = ref("");
const completed = ref("");
const apiKey = ref("");
const probe = ref<ProviderProbe | null>(null);
const preset = ref<ProviderPreset | null>(null);
const catalog = ref<Protocol[]>([]);
const err = ref("");
const errOnSave = ref(false);
const nameErr = ref("");
const composing = ref(false);
const busy = ref(false);
const name = ref("");
const kind = ref("");
const picked = ref<string[]>([]);
// What the probe reported, plus anything typed in. Kept apart from `probe` so
// an added name survives without pretending the endpoint reported it.
const models = ref<string[]>([]);
const facts = ref<Record<string, ModelFact>>({});
const checkingModel = ref("");
const win = ref("");
const maxOut = ref("");
const thinkingOn = ref(true);
const thinkingProtocol = ref("");
const heads = ref("");
const extra = ref("");
const noProxy = ref(false);

// The endpoint says which wires are on the table and the kernel says what
// each one is, so neither list is written here.
onMounted(() => {
  props.port.protocols().then((items) => {
    catalog.value = items;
    kind.value = kind.value || (items.some((p) => p.kind === "openai") ? "openai" : items[0]?.kind ?? "");
  }).catch(() => { catalog.value = []; });
});

const loadPresets = () => props.port.providerPresets();
const choices = computed(() => catalog.value.map((p) => p.kind));
const detectedChoices = computed(() => (probe.value ? (probe.value.kinds?.length ? probe.value.kinds : [probe.value.kind]) : []));
const searchOn = computed(() => detectedChoices.value.filter((k) => catalog.value.find((p) => p.kind === k)?.serverWebSearch));
const searchSplit = computed(() => searchOn.value.length > 0 && searchOn.value.length < detectedChoices.value.length);
const nameBad = computed(() => !composing.value && name.value.trim() !== "" && !sourceNameUsable(name.value));
const nameInvalid = computed(() => nameBad.value || nameErr.value !== "");
const extraBad = computed(() => extra.value.trim() !== "" && parseExtraBody(extra.value) === null);
const protocolCanThink = computed(() => catalog.value.find((p) => p.kind === kind.value)?.reasoningParams ?? false);
const wire = computed(() => catalog.value.find((p) => p.kind === kind.value));
const effortField = computed(() => (wire.value?.effortUnder?.length && !wire.value.effortUnder.includes(thinkingProtocol.value) ? undefined : wire.value?.effortField));
// A wire with no listing shape has nothing to read: the model id is declared
// rather than discovered, and a probe that must fail reads as a broken endpoint.
const protocolCanProbe = computed(() => (catalog.value.find((p) => p.kind === kind.value)?.discovery ?? "") !== "");
const thinkingChoices = computed(() => THINKING.filter(([value]) => value !== "none"));

// A source already at this host changes what a blank key means: another door
// onto that account rather than an account with no credential.
const sibling = computed(() => props.known.find((p) => hostOf(p.baseUrl) === hostOf(baseUrl.value) && baseUrl.value.trim() !== ""));
const siblingName = computed(() => (sibling.value ? vendorLabel(hostOf(sibling.value.baseUrl)) : ""));

const setFacts = (update: (current: Record<string, ModelFact>) => Record<string, ModelFact>) => { facts.value = update(facts.value); };

const connect = async () => {
  busy.value = true;
  err.value = "";
  errOnSave.value = false;
  const typedUrl = baseUrl.value;
  try {
    const got = await props.port.probeProvider(baseUrl.value.trim(), apiKey.value.trim());
    probe.value = got;
    const resolved = got.baseUrl?.trim() ?? "";
    const changed = resolved !== "" && resolved !== baseUrl.value.trim();
    if (changed) baseUrl.value = resolved;
    completed.value = changed ? resolved : "";
    kind.value = kind.value || got.kind;
    models.value = [...new Set([...got.models, ...models.value])];
    picked.value = picked.value.length ? picked.value : got.models.slice(0, 8);
    facts.value = {
      ...Object.fromEntries(got.models.map((model) => [model, { origin: "endpoint" as const }])),
      ...facts.value,
    };
    name.value = name.value.trim() || uniqueName(nameFrom(typedUrl), props.taken);
  } catch (e) {
    err.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const save = async () => {
  busy.value = true;
  err.value = "";
  nameErr.value = "";
  errOnSave.value = true;
  try {
    await props.port.saveProvider({
      name: name.value.trim(),
      kind: kind.value,
      baseUrl: baseUrl.value.trim(),
      apiKey: apiKey.value.trim(),
      models: picked.value,
      default: picked.value[0] ?? "",
      authHeader: probe.value?.authHeader ?? false,
      noProxy: noProxy.value || (probe.value?.noProxy ?? false),
      effort: "",
      vision: (probe.value?.vision ?? []).filter((m) => picked.value.includes(m)),
      contextWindow: numeric(win.value),
      maxOutputTokens: numeric(maxOut.value),
      reasoningProtocol: protocolCanThink.value ? (thinkingOn.value ? thinkingProtocol.value : "none") : undefined,
      headers: parseHeaders(heads.value),
      extraBody: parseExtraBody(extra.value) ?? {},
    });
    props.onDone();
  } catch (e) {
    if (e instanceof HttpError && e.reason?.code && NAME_CODES.has(e.reason.code)) nameErr.value = reason(e);
    else err.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const toggle = (m: string) => {
  picked.value = picked.value.includes(m) ? picked.value.filter((x) => x !== m) : [...picked.value, m];
};

const addModel = (m: string) => {
  models.value = models.value.includes(m) ? models.value : [m, ...models.value];
  picked.value = picked.value.includes(m) ? picked.value : [...picked.value, m];
  facts.value = { ...facts.value, [m]: { origin: "manual" } };
};

const draftKey = computed(() => JSON.stringify([name.value, baseUrl.value, apiKey.value, kind.value, picked.value, win.value, maxOut.value, thinkingOn.value, thinkingProtocol.value, heads.value, extra.value, noProxy.value, probe.value?.authHeader, probe.value?.noProxy]));
const { batch, testAll } = useModelBatch({
  port: () => props.port,
  picked,
  draftKey,
  setFacts,
  origin: "manual",
  blocked: computed(() => checkingModel.value !== "" || busy.value || baseUrl.value.trim() === ""),
  request: computed(() => ({
    baseUrl: baseUrl.value.trim(), apiKey: apiKey.value.trim(), kind: kind.value,
    authHeader: probe.value?.authHeader ?? false, noProxy: noProxy.value || (probe.value?.noProxy ?? false),
  })),
});
const testing = computed(() => checkingModel.value !== "" || batch.value === "running");

const checkModel = async (model: string) => {
  if (testing.value || busy.value) return;
  checkingModel.value = model;
  facts.value = {
    ...facts.value,
    [model]: { ...(facts.value[model] ?? { origin: "manual" }), checking: true },
  };
  try {
    const got = await props.port.checkProviderModel({
      model,
      baseUrl: baseUrl.value.trim(),
      apiKey: apiKey.value.trim(),
      kind: kind.value,
      authHeader: probe.value?.authHeader ?? false,
      noProxy: noProxy.value || (probe.value?.noProxy ?? false),
    });
    facts.value = {
      ...facts.value,
      [model]: checkedFact(facts.value[model], "manual", got),
    };
  } catch {
    facts.value = {
      ...facts.value,
      [model]: { ...(facts.value[model] ?? { origin: "manual" }), status: "unknown", reason: "network" },
    };
  } finally {
    checkingModel.value = "";
    facts.value = {
      ...facts.value,
      [model]: { ...(facts.value[model] ?? { origin: "manual" }), checking: false },
    };
  }
};

const changeKind = (e: Event) => {
  kind.value = (e.target as HTMLSelectElement).value;
  facts.value = clearModelCheckFacts(facts.value);
};

const changeEndpoint = (e: Event) => {
  const value = (e.target as HTMLInputElement).value;
  if (value.trim() !== baseUrl.value.trim()) {
    probe.value = null;
    completed.value = "";
  }
  baseUrl.value = value;
  facts.value = clearModelCheckFacts(facts.value);
};

const changeKey = (e: Event) => {
  apiKey.value = (e.target as HTMLInputElement).value;
  facts.value = clearModelCheckFacts(facts.value);
};

const changeName = (e: Event) => {
  name.value = (e.target as HTMLInputElement).value;
  nameErr.value = "";
};

const toggleProxy = () => {
  noProxy.value = !noProxy.value;
  facts.value = clearModelCheckFacts(facts.value);
};

function digits(value: string): string { return value.replace(/\D/g, ""); }
function numeric(value: string): number { return value.trim() ? Number(value) : 0; }

// A second provider from the same vendor must not overwrite the first.
function uniqueName(base: string, taken: string[]): string {
  if (!taken.includes(base)) return base;
  for (let i = 2; i < 100; i++) {
    if (!taken.includes(`${base}-${i}`)) return `${base}-${i}`;
  }
  return base;
}
</script>

<template>
  <PresetInstall v-if="preset" :port="port" :preset="preset" :on-done="onDone" :on-cancel="onCancel" :on-back="() => (preset = null)" />
  <fieldset v-else class="addp" :disabled="busy && errOnSave">
    <div class="addp-head">
      <div>
        <span class="step">{{ t("models.custom_source") }}</span>
        <strong>{{ t("models.add_a_model_source") }}</strong>
        <p>{{ t("models.use_the_service_documentation_connection") }}</p>
      </div>
      <button class="addp-close" @click="onCancel" :disabled="busy || testing" :aria-label="t('common.cancel')"><StudioIcon name="close" /></button>
    </div>
    <PresetGallery v-if="baseUrl.trim() === ''" :load="loadPresets" :on-pick="(p) => (preset = p)" :disabled="busy || checkingModel !== ''" />
    <div class="fields">
      <label class="grow">
        <span>{{ t("models.api_protocol") }}</span>
        <select :aria-label="t('models.api_protocol')" data-action="provider.draft" data-value="protocol" :value="kind" :disabled="busy || testing" @change="changeKind">
          <option v-for="k in choices" :key="k" :value="k">{{ t(KIND_LABEL[k] ?? k) }}</option>
        </select>
        <i class="tip">{{ t("models.follow_the_provider_documentation_a") }}</i>
      </label>
      <label class="grow">
        <span>{{ t("models.endpoint_address") }}</span>
        <input
          :aria-label="t('models.endpoint_address')"
          data-action="provider.draft" data-value="endpoint"
          :value="baseUrl"
          placeholder="https://api.moonshot.cn/v1"
          spellcheck="false"
          :disabled="busy || testing"
          @input="changeEndpoint"
        />
        <i v-if="wire?.answers === 'decision'" class="tip">{{ t("models.enter_the_services_base_address") }}</i>
      </label>
      <label class="grow full">
        <span>API Key{{ t(sibling ? "models.leave_blank_to_keep_the" : "") }}</span>
        <input type="password" data-action="provider.draft" data-value="credential" :value="apiKey" :placeholder="sibling ? '········' : ''" spellcheck="false" :disabled="busy || testing" @input="changeKey" />
      </label>
      <p class="addp-privacy">{{ t("common.the_api_key_is_stored") }}</p>
      <div class="grow full name-cell">
        <label>
          <span>{{ t("common.source_name") }}</span>
          <input
            :aria-label="t('common.source_name')"
            data-action="provider.draft" data-value="name"
            :value="name"
            :placeholder="t('models.made_after_connecting_editable')"
            spellcheck="false"
            :disabled="busy || testing"
            :aria-invalid="nameInvalid || undefined"
            aria-describedby="addp-name-rule"
            @input="changeName"
            @compositionstart="composing = true"
            @compositionend="composing = false"
          />
        </label>
        <p id="addp-name-rule" class="name-rule" :data-bad="nameInvalid || undefined" aria-live="polite">
          {{ nameErr !== "" ? nameErr : t("models.use_only_letters_digits_dots") }}
        </p>
      </div>
      <p v-if="sibling" class="acct-note">
        {{ t("models.this_address_already_holds_name", { name: siblingName }) }}{{ t("models.this_adds_a_second_way") }}
      </p>
    </div>

    <div class="addp-section">
      <div class="addp-section-head">
        <div><strong>{{ t("models.model_limits") }}</strong><span>{{ t("models.defaults_for_models_from_this") }}</span></div>
      </div>
      <div class="fields token-limits">
        <label class="grow">
          <span>{{ t("common.context_window") }}</span>
          <span class="unit-field"><input :aria-label="t('common.context_window')" data-action="provider.draft" data-value="context-window" inputmode="numeric" :value="win" placeholder="128000" @input="win = digits(($event.target as HTMLInputElement).value)" /><i>tokens</i></span>
          <i class="tip">{{ t("models.the_shared_capacity_for_input") }}</i>
        </label>
        <label class="grow">
          <span>{{ t("models.maximum_output") }}</span>
          <span class="unit-field"><input :aria-label="t('models.maximum_output')" data-action="provider.draft" data-value="max-output" inputmode="numeric" :value="maxOut" :placeholder="t('common.auto')" @input="maxOut = digits(($event.target as HTMLInputElement).value)" /><i>tokens</i></span>
          <i class="tip">{{ t("models.the_per_turn_reply_limit") }}</i>
        </label>
      </div>
    </div>

    <details class="addp-options">
      <summary>
        <span class="tx">
          <strong>{{ t("models.advanced_connection_options") }}</strong>
          <small>{{ t("models.thinking_controls_proxy_and_gateway") }}</small>
        </span>
        <span class="summary-value">{{ t("common.optional") }}</span>
      </summary>
      <div class="addp-options-body">
        <div class="setting-line">
          <span class="setting-copy">
            <strong>{{ t("models.send_thinking_controls") }}</strong>
            <small>{{ t(protocolCanThink ? "models.allow_this_protocol_to_send" : "models.this_protocol_has_no_thinking") }}</small>
          </span>
          <button
            type="button"
            class="switch-control"
            data-action="provider.draft" data-value="thinking"
            role="switch"
            :aria-label="t('models.send_thinking_controls')"
            :aria-checked="protocolCanThink && thinkingOn"
            :disabled="!protocolCanThink || busy || testing"
            @click="thinkingOn = !thinkingOn"
          ><span /></button>
        </div>
        <label v-if="protocolCanThink && thinkingOn" class="advanced-field">
          <span>{{ t("models.thinking_protocol") }}</span>
          <select :value="thinkingProtocol" @change="thinkingProtocol = ($event.target as HTMLSelectElement).value">
            <option v-for="[value, label] in thinkingChoices" :key="value" :value="value">{{ t(label) }}</option>
          </select>
          <i>{{ t("models.use_auto_to_follow_the") }}</i>
          <EffortShape :field="effortField" />
        </label>
        <div class="setting-line">
          <span class="setting-copy">
            <strong>{{ t("models.bypass_system_proxy") }}</strong>
            <small>{{ t("models.enable_only_when_the_endpoint") }}</small>
          </span>
          <button type="button" class="switch-control" data-action="provider.draft" data-value="no-proxy" role="switch" :aria-label="t('models.bypass_system_proxy')" :aria-checked="noProxy || (probe?.noProxy ?? false)"
            :disabled="busy || testing || probe?.noProxy === true" @click="toggleProxy"><span /></button>
        </div>
        <label class="advanced-field">
          <span>{{ t("models.extra_request_headers") }}</span>
          <textarea rows="3" :value="heads" spellcheck="false" :placeholder="HEADERS_PLACEHOLDER" @input="heads = ($event.target as HTMLTextAreaElement).value" />
          <i>{{ t("models.one_name_value_pair_per") }}</i>
        </label>
        <label class="advanced-field">
          <span>{{ t("models.extra_request_body") }}</span>
          <textarea rows="4" :value="extra" spellcheck="false" :aria-invalid="extraBad || undefined" :placeholder="EXTRA_PLACEHOLDER" @input="extra = ($event.target as HTMLTextAreaElement).value" />
          <i>{{ t("models.only_for_optional_fields_required") }}</i>
          <em v-if="extraBad" class="field-error">{{ t("models.enter_a_valid_json_object") }}</em>
        </label>
      </div>
    </details>

    <div class="addp-section">
      <div class="addp-section-head">
        <div>
          <strong>{{ t("models.model_catalogue") }}</strong>
          <span>{{ t(protocolCanProbe ? "models.add_at_least_one_model" : "models.this_protocol_has_no_model") }}</span>
        </div>
        <button v-if="protocolCanProbe" class="act quiet" data-action="provider.probe" @click="connect" :disabled="busy || testing || baseUrl.trim() === ''">
          {{ t(busy ? "models.checking_2" : "models.check_connection_and_read_models") }}
        </button>
      </div>
      <div v-if="err && !errOnSave" class="find" data-lvl="warn" role="alert">
        <span class="t">{{ t("models.unreachable") }}</span>
        <span class="why">{{ err }}</span>
      </div>
      <p v-if="probe" class="probe-ok">{{ t("models.connection_works_n_models_found", { n: probe.models.length }) }}</p>
      <p v-if="completed !== '' && completed === baseUrl.trim()" class="probe-ok">{{ t("common.address_completed_to_url", { url: completed }) }}</p>
      <div class="mlist">
        <div class="mlhead">
          <span class="ttl">{{ t("models.enabled_models") }}</span>
          <span class="count">{{ t("models.on_of_all_enabled", { on: picked.length, all: models.length }) }}</span>
          <button class="mrefresh" data-action="provider.model-check-all" @click="testAll"
            :disabled="busy || testing || picked.length === 0 || baseUrl.trim() === ''" :aria-busy="batch === 'running' || undefined">
            {{ t("models.test_enabled_models_n", { n: picked.length }) }}
          </button>
        </div>
        <p class="mguide">{{ t("models.the_catalog_is_for_discovery") }}</p>
        <p v-if="batch === 'stopped'" class="mdiff" role="status">{{ t("models.no_further_checks_started_the") }}</p>
        <ModelChoice
          :models="models"
          :picked="picked"
          :vision="probe?.vision ?? []"
          :facts="facts"
          :on-check="checkModel"
          :check-disabled="busy || testing || baseUrl.trim() === ''"
          :on-toggle="toggle"
          :on-add="addModel"
        />
      </div>
    </div>

    <template v-if="probe && (probe.ambiguous || probe.noProxy || searchSplit)">
      <p v-if="searchSplit" class="acct-note">
        {{ t("models.this_address_answers_on_both", {
          on: searchOn.map((k) => t(KIND_LABEL[k] ?? k)).join("、"),
        }) }}
      </p>
      <p v-if="probe.ambiguous" class="acct-note">
        {{ t("common.both_access_methods_answer_with") }}
      </p>
      <p v-if="probe.noProxy" class="acct-note">{{ t("models.the_proxy_could_not_reach") }}</p>
    </template>
    <div class="acts-bar">
      <div v-if="err && errOnSave" class="find" data-lvl="err" role="alert">
        <span class="t">{{ t("common.could_not_save") }}</span>
        <span class="why">{{ err }}</span>
      </div>
      <div class="acts addp-footer">
        <button class="act" data-action="provider.add" data-primary @click="save" :aria-describedby="nameBad ? 'addp-name-rule' : undefined" :disabled="busy || composing || testing || picked.length === 0 || name.trim() === '' || nameBad || kind === '' || baseUrl.trim() === '' || extraBad">
          {{ t(busy ? "common.saving" : "common.add_source") }}
        </button>
        <button class="act" @click="onCancel" :disabled="busy || testing">{{ t("common.cancel") }}</button>
      </div>
    </div>
  </fieldset>
</template>
