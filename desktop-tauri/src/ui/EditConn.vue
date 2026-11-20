<script setup lang="ts">
import { computed, h, onMounted, onScopeDispose, ref, watch } from "vue";
import { t } from "../i18n";
import type { ModelEffort, ModelLimit, Protocol, ProviderCheck, ProviderEdit, ProviderEntry } from "../port/port";
import ModelChoice, { checkedFact, clearModelCheckFacts, type ModelFact } from "./ModelChoice.vue";
import type { Port } from "./Providers.vue";
import { savedNotApplied, reason, say } from "../i18n/kernel";
import { checkFailure } from "./provider_check";
import { IDLE_TIMEOUT_MAX, IDLE_TIMEOUT_MIN, THINKING, headerLines, parseEffortLevels, parseExtraBody, parseHeaders, parseIdleTimeout } from "./provider_compat";
import ModelEfforts from "./ModelEfforts.vue";
import EffortShape from "./EffortShape.vue";
import { KIND_LABEL } from "./vendors";
import ModelLimits, { limitTextOf, limitsToSend, type LimitText } from "./ModelLimits.vue";
import ModelPrices, { incomplete, priceTextOf, pricesToSend, sourceBad, type PriceText } from "./ModelPrices.vue";
import BillingModeField, { PAYG } from "./BillingModeField.vue";
import { useModelBatch } from "./useModelBatch";
import EndpointSwitches, { hasEndpointSwitches } from "./EndpointSwitches.vue";
import BalanceFields from "./BalanceFields.vue";

// Only what this form owns is sent: the entry keeps its prices, effort
// vocabularies and everything else the panel cannot show. declare opens the
// form on the reasoning fields: the composer sends a user here when the
// endpoint reported no effort levels.
const props = withDefaults(defineProps<{
  entry: ProviderEntry;
  label?: string;
  wireFixed?: boolean;
  initialCheck?: ProviderCheck;
  port: Port;
  busy: string;
  setBusy: (b: string) => void;
  onDone: () => void | Promise<void>;
  onRevert: () => void;
  onSaved?: () => void;
  onDirty?: (dirty: boolean) => void;
  declare?: boolean;
  justSaved?: boolean;
}>(), { wireFixed: false, declare: false, justSaved: false });

const HEADERS_PLACEHOLDER = "HTTP-Referer: https://example.com\nX-Title: Reasonix";
const EXTRA_PLACEHOLDER = '{\n  "enable_thinking": true\n}';

const label = computed(() => props.label ?? props.entry.name);
const seededModels = [...new Set([...props.entry.models, ...(props.initialCheck?.models ?? [])])];
const seededVision = [...new Set([...(props.entry.visionModels ?? []), ...(props.initialCheck?.vision ?? [])])];
const baseUrl = ref(props.entry.baseUrl);
const kind = ref(props.entry.kind);
const wires = ref<Protocol[]>([]);
onMounted(() => {
  Promise.resolve().then(() => props.port.protocols()).then((items) => { wires.value = items; }).catch(() => { wires.value = []; });
});
const sameRole = (p: Protocol) => (p.answers ?? "chat") === (wires.value.find((w) => w.kind === props.entry.kind)?.answers ?? "chat");
// An account with a sibling door already switches between wires above;
// retyping one entry would make two doors of one kind.
const kinds = computed(() => (props.wireFixed ? [] : wires.value.some((w) => w.kind === props.entry.kind) ? wires.value.filter(sameRole).map((w) => w.kind) : []));
const completed = ref("");
const apiKey = ref("");
const models = ref<string[]>(seededModels);
const picked = ref<string[]>(props.entry.models);
const aliases = ref<Record<string, string>>(props.entry.modelAliases ?? {});
const hidePrefix = ref(props.entry.hideModelPrefix === true);
const vision = ref<string[]>(seededVision);
const visionSettable = ref<string[] | undefined>(
  props.entry.visionSettable ? [...new Set([...props.entry.visionSettable, ...(props.initialCheck?.vision ?? [])])] : undefined,
);
const facts = ref<Record<string, ModelFact>>(modelFacts(props.entry.models, props.initialCheck));
const diff = ref<{ fresh: boolean; added: number; missing: number } | null>(
  props.initialCheck?.ok ? catalogDiff(props.entry.models, props.initialCheck.models ?? [], false) : null,
);
const checkingModel = ref("");
const def = ref(props.entry.default || props.entry.models[0] || "");
const err = ref<{ text: string; kind: "save" | "unapplied" } | null>(null);
const refreshFail = ref("");
const more = ref(props.declare);
const win = ref(props.entry.contextWindow ? String(props.entry.contextWindow) : "");
const maxOut = ref(props.entry.maxOutputTokens ? String(props.entry.maxOutputTokens) : "");
const idleText = ref(props.entry.idleTimeoutSeconds ? String(props.entry.idleTimeoutSeconds) : "");
const idle = computed(() => parseIdleTimeout(idleText.value));
const idleSecs = computed(() => { const state = idle.value; return state.ok ? state.secs : 0; });
const limitText = ref<Record<string, LimitText>>(
  Object.fromEntries(Object.entries(props.entry.modelLimits ?? {}).map(([m, l]) => [m, limitTextOf(l)])),
);
const priceText = ref<Record<string, PriceText>>(
  Object.fromEntries(Object.entries(props.entry.modelPrices ?? {}).map(([m, r]) => [m, priceTextOf(r)])),
);
const billing = ref(props.entry.billingMode || PAYG);
const balUrl = ref(props.entry.balanceUrl ?? "");
const balFormat = ref(props.entry.balanceFormat ?? "");
const think = ref(props.entry.reasoningProtocol ?? "");
const search = ref(props.entry.webSearch ?? false);
const carry = ref(props.entry.continuation ?? "");
// These belong to the stored wire; a draft that retypes it carries none of them.
const wireSwitches = computed(() => kind.value === props.entry.kind);
const draftWire = computed(() => wires.value.find((w) => w.kind === kind.value));
// Until saved, the thinking field follows the wire being picked: the stored
// entry's field belongs to the stored wire.
const effortField = computed(() => {
  const draft = draftWire.value;
  return kind.value === props.entry.kind || !draft
    ? props.entry.effortField
    : !draft.effortUnder?.length || think.value === "" || draft.effortUnder.includes(think.value) ? draft.effortField : undefined;
});
const levelText = ref((props.entry.supportedEfforts ?? []).join(", "));
const defEffort = ref(props.entry.defaultEffort ?? "");
const levels = computed(() => parseEffortLevels(levelText.value));
const ownEfforts = ref<Record<string, ModelEffort>>(props.entry.modelEfforts ?? {});
// Kimi K3 carries a fixed vocabulary and "none" sends no reasoning field, so
// a declared list is kept on file but has nothing to act on under either.
const levelsDormant = computed(() => think.value === "kimi-k3" || think.value === "none");
const reasoning = ref<HTMLSelectElement | null>(null);
const revealReasoning = () => {
  reasoning.value?.scrollIntoView({ block: "center" });
  reasoning.value?.focus({ preventScroll: true });
};
onMounted(() => { if (props.declare) revealReasoning(); });
watch(() => props.declare, (on) => { if (on) revealReasoning(); });
const heads = ref(headerLines(props.entry.headers));
const extra = ref(props.entry.extraBody ? JSON.stringify(props.entry.extraBody, null, 2) : "");
const saving = computed(() => props.busy === `edit:${props.entry.name}`);
const refreshing = computed(() => props.busy === `refresh:${props.entry.name}`);
const listable = computed(() => props.entry.canListModels !== false);
const extraBad = computed(() => extra.value.trim() !== "" && parseExtraBody(extra.value) === null);

const setFacts = (update: (current: Record<string, ModelFact>) => Record<string, ModelFact>) => { facts.value = update(facts.value); };

const toggle = (list: string[], set: (v: string[]) => void, m: string) =>
  set(list.includes(m) ? list.filter((x) => x !== m) : [...list, m]);

// Which rows may take images. The kernel answers per model now — one endpoint
// serves an image-taking model beside text-only ones — so an older kernel that
// only sends the connection-wide boolean still gets the old answer.
const visionLocked = (m: string) =>
  (visionSettable.value ? !visionSettable.value.includes(m) : props.entry.canSetVision === false);

// A name the endpoint never reported. It lands ticked because typing it out
// is already the answer to "do you want this one", and at the head of the
// list because a new row three hundred names down reads as nothing happening.
const addModel = (m: string) => {
  models.value = models.value.includes(m) ? models.value : [m, ...models.value];
  picked.value = picked.value.includes(m) ? picked.value : [...picked.value, m];
  facts.value = { ...facts.value, [m]: { origin: "manual" } };
};

// Re-asking the endpoint is how a source that gained models catches up; the
// ticks the user already made survive it. A blank key field means "keep the
// stored one", so the kernel is asked through the saved source with the form's
// address, key and protocol laid over it: what is being edited is what gets tested.
const refetch = async () => {
  props.setBusy(`refresh:${props.entry.name}`);
  err.value = null;
  refreshFail.value = "";
  diff.value = null;
  try {
    const refreshed = await props.port.checkProvider(props.entry.name, { baseUrl: baseUrl.value.trim(), apiKey: apiKey.value.trim(), kind: kind.value });
    if (!refreshed.ok) {
      refreshFail.value = checkFailure(refreshed);
      return;
    }
    const changed = !!refreshed.baseUrl && refreshed.baseUrl !== baseUrl.value.trim();
    if (changed) baseUrl.value = refreshed.baseUrl!;
    completed.value = changed ? refreshed.baseUrl! : "";
    const found = refreshed.models ?? [];
    if (found.length === 0) {
      refreshFail.value = say({ code: "provider.probe.no_chat_models", params: { count: 0 } });
      return;
    }
    const readers = refreshed.vision ?? [];
    const current = models.value;
    diff.value = catalogDiff(current, found, true);
    setFacts((factsNow) => refreshedFacts(current, found, factsNow));
    models.value = [...new Set([...found, ...current])];
    vision.value = [...new Set([...vision.value, ...readers])];
    if (visionSettable.value) visionSettable.value = [...new Set([...visionSettable.value, ...readers])];
  } catch (e) {
    refreshFail.value = reason(e);
  } finally {
    props.setBusy("");
  }
};

const checkModel = async (model: string) => {
  if (checkingModel.value || props.busy) return;
  checkingModel.value = model;
  facts.value = {
    ...facts.value,
    [model]: { ...(facts.value[model] ?? { origin: "configured" }), checking: true },
  };
  try {
    const got = await props.port.checkProviderModel({
      name: props.entry.name,
      model,
      baseUrl: baseUrl.value.trim(),
      apiKey: apiKey.value.trim(),
      kind: kind.value,
    });
    facts.value = {
      ...facts.value,
      [model]: checkedFact(facts.value[model], "configured", got),
    };
  } catch {
    facts.value = {
      ...facts.value,
      [model]: { ...(facts.value[model] ?? { origin: "configured" }), status: "unknown", reason: "network" },
    };
  } finally {
    checkingModel.value = "";
    facts.value = {
      ...facts.value,
      [model]: { ...(facts.value[model] ?? { origin: "configured" }), checking: false },
    };
  }
};

// A connection list typed here is what every inheriting model gets; with none
// typed, the kernel's answer holds only while the form still matches the saved one.
const savedLevels = (props.entry.supportedEfforts ?? []).join(",");
const inheritedFor = (model: string): ModelEffort | null | undefined => {
  if (levels.value.length > 0 && !levelsDormant.value) {
    return { supportedEfforts: levels.value, defaultEffort: levels.value.includes(defEffort.value) ? defEffort.value : "" };
  }
  if (think.value !== (props.entry.reasoningProtocol ?? "") || levels.value.join(",") !== savedLevels) return null;
  return props.entry.inheritedEfforts?.[model];
};

// A provider-wide value typed here is what every model without its own
// inherits; until it is saved, the kernel's answer holds only for what was saved.
const inheritedLimitsFor = (model: string): ModelLimit | null | undefined => {
  const typedWin = Number(win.value) || 0;
  const typedOut = Number(maxOut.value) || 0;
  const base = props.entry.inheritedLimits?.[model];
  const winMoved = typedWin !== (props.entry.contextWindow ?? 0);
  const outMoved = typedOut !== (props.entry.maxOutputTokens ?? 0);
  return {
    contextWindow: winMoved ? typedWin || undefined : base?.contextWindow,
    maxOutputTokens: outMoved ? typedOut || undefined : base?.maxOutputTokens,
  };
};

const shapeWhy = computed(() => h(EffortShape, {
  field: think.value === (props.entry.reasoningProtocol ?? "") || kind.value !== props.entry.kind ? effortField.value : undefined,
  level: levels.value[0],
  className: "why",
}));

const draft = (): ProviderEdit => ({
  name: props.entry.name,
  ...(kind.value !== props.entry.kind && { kind: kind.value }),
  baseUrl: baseUrl.value.trim(),
  apiKey: apiKey.value.trim(),
  models: picked.value,
  ...(hidePrefix.value !== (props.entry.hideModelPrefix === true) && { hideModelPrefix: hidePrefix.value }),
  modelAliases: Object.fromEntries(Object.entries(aliases.value).filter(([m, value]) => picked.value.includes(m) && value.trim()).map(([m, value]) => [m, value.trim()])),
  default: picked.value.includes(def.value) ? def.value : picked.value[0] ?? "",
  vision: vision.value.filter((m) => picked.value.includes(m)),
  contextWindow: Number(win.value.replace(/\D/g, "")) || 0,
  maxOutputTokens: Number(maxOut.value.replace(/\D/g, "")) || 0,
  idleTimeoutSeconds: idle.value.ok ? idle.value.secs : 0,
  ...(billing.value !== (props.entry.billingMode || PAYG) && { billingMode: billing.value }),
  ...(balUrl.value.trim() !== (props.entry.balanceUrl ?? "") && { balanceUrl: balUrl.value.trim() }),
  ...(balFormat.value !== (props.entry.balanceFormat ?? "") && { balanceFormat: balFormat.value }),
  reasoningProtocol: think.value,
  ...(wireSwitches.value && props.entry.canWebSearch && search.value !== (props.entry.webSearch ?? false) && { webSearch: search.value }),
  ...(wireSwitches.value && props.entry.canSetContinuation && carry.value !== (props.entry.continuation ?? "") && { continuation: carry.value }),
  supportedEfforts: levels.value,
  defaultEffort: levels.value.includes(defEffort.value) ? defEffort.value : "",
  modelEfforts: Object.fromEntries(picked.value.filter((m) => ownEfforts.value[m]).map((m) => [m, ownEfforts.value[m]])),
  modelLimits: limitsToSend(picked.value, limitText.value, props.entry.modelLimits ?? {}),
  modelPrices: pricesToSend(picked.value, priceText.value, props.entry.modelPrices ?? {}),
  headers: parseHeaders(heads.value),
  extraBody: parseExtraBody(extra.value) ?? {},
});

// What a save would send, with order and unparsable text normalised: ticking
// a row off and on again, or typing a value back, is no change.
const fingerprint = (readers: string[] = vision.value) => JSON.stringify({
  ...draft(), models: [...picked.value].sort(), vision: readers.filter((m) => picked.value.includes(m)).sort(),
  idle: idle.value.ok ? idle.value.secs : idleText.value, extra: parseExtraBody(extra.value) ?? extra.value,
});
// The baseline is the saved configuration; image support a connection test
// seeded into the form is a change until it is saved.
const stored = ref(fingerprint(props.entry.visionModels ?? []));
const draftKey = computed(() => fingerprint());
const dirty = computed(() => draftKey.value !== stored.value);
watch(dirty, (d) => props.onDirty?.(d), { immediate: true });
onScopeDispose(() => props.onDirty?.(false));
const { batch, testAll } = useModelBatch({
  port: () => props.port,
  picked,
  draftKey,
  setFacts,
  origin: "configured",
  blocked: computed(() => checkingModel.value !== "" || props.busy !== ""),
  request: computed(() => ({ name: props.entry.name, baseUrl: baseUrl.value.trim(), apiKey: apiKey.value.trim(), kind: kind.value })),
});
const testing = computed(() => checkingModel.value !== "" || batch.value === "running");
const edited = ref(false);
watch(dirty, (d) => { if (d) edited.value = true; }, { immediate: true });

const canSave = computed(() => (dirty.value || err.value?.kind === "unapplied") && props.busy === "" && !testing.value && picked.value.length > 0 && !extraBad.value && idle.value.ok && !picked.value.some((m) => incomplete(priceText.value[m]) || sourceBad(priceText.value[m])));
const phase = computed<Phase>(() => saving.value ? "saving"
  : err.value?.kind === "save" && dirty.value ? "failed"
    : dirty.value ? "dirty"
      : err.value?.kind === "unapplied" ? "unapplied"
        : props.justSaved && !edited.value ? "saved" : "clean");

const save = async () => {
  props.setBusy(`edit:${props.entry.name}`);
  err.value = null;
  const sent = fingerprint();
  try {
    await props.port.editProvider(draft());
    await props.onDone();
  } catch (e) {
    const unapplied = savedNotApplied(e);
    err.value = { text: reason(e), kind: unapplied ? "unapplied" : "save" };
    // Saved but not yet applied: the list has to show what is on file while
    // the form stays open to say why.
    if (unapplied) {
      stored.value = sent;
      props.onSaved?.();
    }
  } finally {
    props.setBusy("");
  }
};

const keySave = (e: KeyboardEvent) => {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.key.toLowerCase() !== "s") return;
  e.preventDefault();
  if (canSave.value) void save();
};

const changeKind = (e: Event) => {
  kind.value = (e.target as HTMLSelectElement).value;
  facts.value = clearModelCheckFacts(facts.value);
};

const changeEndpoint = (e: Event) => {
  baseUrl.value = (e.target as HTMLInputElement).value;
  facts.value = clearModelCheckFacts(facts.value);
  refreshFail.value = "";
  diff.value = null;
};

const changeKey = (e: Event) => {
  apiKey.value = (e.target as HTMLInputElement).value;
  facts.value = clearModelCheckFacts(facts.value);
  refreshFail.value = "";
  diff.value = null;
};

const setThinking = (send: boolean) => {
  think.value = send ? (think.value === "none" ? (props.entry.reasoningProtocol === "none" ? "" : props.entry.reasoningProtocol ?? "") : think.value) : "none";
};

const setupMore = (e: Event) => { more.value = (e.currentTarget as HTMLDetailsElement).open; };

const openSource = (url: string) => void Promise.resolve(props.port.openExternal(url)).catch(() => {});

const summary = computed(() => compatSummary(think.value, heads.value, extra.value, levels.value.length, picked.value.filter((m) => ownEfforts.value[m]?.supportedEfforts.length).length, idleSecs.value));

function modelFacts(configured: string[], check?: ProviderCheck): Record<string, ModelFact> {
  const found = new Set(check?.ok ? check.models ?? [] : []);
  const out: Record<string, ModelFact> = {};
  for (const model of configured) out[model] = { origin: check?.ok ? (found.has(model) ? "endpoint" : "missing") : "configured" };
  for (const model of found) if (!out[model]) out[model] = { origin: "endpoint" };
  return out;
}

function refreshedFacts(list: string[], found: string[], current: Record<string, ModelFact>): Record<string, ModelFact> {
  const listed = new Set(found);
  const out = { ...current };
  for (const model of [...new Set([...found, ...list])]) {
    const previous = current[model];
    out[model] = {
      ...previous,
      origin: listed.has(model) ? "endpoint" : previous?.origin === "manual" ? "manual" : "missing",
    };
  }
  return out;
}

function catalogDiff(before: string[], found: string[], fresh: boolean) {
  const had = new Set(before);
  const now = new Set(found);
  return {
    fresh,
    added: found.filter((model) => !had.has(model)).length,
    missing: before.filter((model) => !now.has(model)).length,
  };
}

function compatSummary(think: string, heads: string, extra: string, levelCount: number, ownModels: number, idleSecs: number): string {
  const parts: string[] = [];
  const protocol = THINKING.find(([value]) => value === think);
  if (think && protocol) parts.push(t(protocol[1]));
  if (levelCount) parts.push(t("models.n_levels", { n: levelCount }));
  if (ownModels) parts.push(t("models.n_models_with_their_own", { n: ownModels }));
  if (idleSecs) parts.push(t("models.n_s_no_answer_timeout", { n: idleSecs }));
  const headCount = Object.keys(parseHeaders(heads)).length;
  if (headCount) parts.push(t("models.n_headers", { n: headCount }));
  const body = parseExtraBody(extra);
  if (body && Object.keys(body).length) parts.push(t("models.extra_body"));
  return parts.join(" · ");
}

type Phase = "clean" | "dirty" | "saving" | "saved" | "unapplied" | "failed";
const PHASE_TEXT: Record<Phase, string> = {
  clean: "models.no_changes", dirty: "models.unsaved_changes", failed: "models.unsaved_changes", saving: "common.saving_2",
  saved: "models.saved", unapplied: "common.saved_not_applied_yet",
};
</script>

<template>
  <fieldset class="addp" data-edit :disabled="saving" data-action-keydown="provider.save" @keydown="keySave">
    <div class="fields">
      <label v-if="kinds.length > 1" class="grow full">
        <span>{{ t("models.api_protocol") }}</span>
        <select :aria-label="t('models.api_protocol')" data-action="provider.draft" data-value="protocol" :value="kind" :disabled="busy !== '' || checkingModel !== ''" @change="changeKind">
          <option v-for="k in kinds" :key="k" :value="k">{{ t(KIND_LABEL[k] ?? k) }}</option>
        </select>
        <i class="tip">{{ t("models.follow_the_provider_documentation_a") }}</i>
      </label>
      <label class="grow full">
        <span>{{ t("models.endpoint_address") }}</span>
        <input :value="baseUrl" spellcheck="false" :disabled="busy !== '' || checkingModel !== ''" @input="changeEndpoint" />
      </label>
      <label class="grow full">
        <span>{{ t("models.api_key_leave_blank_to") }}</span>
        <input type="password" :value="apiKey" placeholder="········" spellcheck="false" :disabled="busy !== '' || checkingModel !== ''" @input="changeKey" />
      </label>
    </div>

    <div class="addp-section compact">
      <div class="addp-section-head">
        <div><strong>{{ t("models.model_limits") }}</strong><span>{{ t("models.use_the_model_documentation_instead") }}</span></div>
      </div>
      <div class="fields token-limits">
        <label class="grow">
          <span>{{ t("common.context_window") }}</span>
          <span class="unit-field"><input :aria-label="t('common.context_window')" inputmode="numeric" :value="win" :placeholder="t('common.not_declared')" @input="win = ($event.target as HTMLInputElement).value.replace(/\D/g, '')" /><i>tokens</i></span>
          <i class="tip">{{ t("models.the_total_input_tool_results") }}</i>
        </label>
        <label class="grow">
          <span>{{ t("models.maximum_output") }}</span>
          <span class="unit-field"><input :aria-label="t('models.maximum_output')" inputmode="numeric" :value="maxOut" :placeholder="t('common.auto')" @input="maxOut = ($event.target as HTMLInputElement).value.replace(/\D/g, '')" /><i>tokens</i></span>
          <i class="tip">{{ t("models.the_per_turn_generation_limit") }}</i>
        </label>
      </div>
      <ModelLimits v-if="picked.length > 1 || picked.some((m) => entry.modelLimits?.[m])" :models="picked" :value="limitText" :on-change="(next) => (limitText = next)" :inherited="inheritedLimitsFor" :labels="entry.modelShorts" />
      <ModelPrices v-if="picked.length > 0" :models="picked" :value="priceText" :on-change="(next) => (priceText = next)" :official="entry.officialPricing === true" :stored="entry.modelPrices ?? {}" :labels="entry.modelShorts"
        :on-open-source="openSource" />
      <div v-if="picked.length > 0" class="fields"><BillingModeField :value="billing" :on-change="(v) => (billing = v)" /></div>
    </div>

    <div class="vway" data-field="hide-model-prefix">
      <span class="lb">{{ t("models.hide_the_shared_prefix") }}</span>
      <div class="seg" role="group" :aria-label="t('models.model_name_prefix_of_name', { name: entry.displayName?.trim() || entry.name })">
        <button v-for="on in [true, false]" :key="String(on)" type="button" data-action="provider.draft" :data-value="`hide-model-prefix:${on}`"
          :aria-pressed="hidePrefix === on" :disabled="busy !== ''" @click="hidePrefix = on">
          {{ t(on ? "models.on" : "common.off") }}
        </button>
      </div>
      <span class="why">{{ t("models.the_same_prefix_a_routing") }}</span>
    </div>

    <div class="mlist">
      <div class="mlhead">
        <span class="ttl">{{ t("common.models") }}</span>
        <span class="count">{{ t("models.on_of_all_enabled", { on: picked.length, all: models.length }) }}</span>
        <button v-if="listable" class="mrefresh" data-action="provider.probe" @click="refetch" :disabled="busy !== '' || testing"
          :title="t('models.read_the_model_list_from')">
          {{ t(refreshing ? "common.reading_from_provider" : "models.read_available_models_from_provider") }}
        </button>
        <button class="mrefresh" data-action="provider.model-check-all" @click="testAll"
          :disabled="busy !== '' || testing || picked.length === 0" :aria-busy="batch === 'running' || undefined">
          {{ t("models.test_enabled_models_n", { n: picked.length }) }}
        </button>
      </div>
      <p class="mguide">
        {{ t("models.the_catalog_is_for_discovery") }}
      </p>
      <p v-if="completed !== '' && completed === baseUrl.trim()" class="mdiff" role="status">{{ t("common.address_completed_to_url", { url: completed }) }}</p>
      <p v-if="batch === 'stopped'" class="mdiff" role="status">{{ t("models.no_further_checks_started_the") }}</p>
      <p v-if="diff && diff.fresh && diff.added === 0 && diff.missing === 0" class="mdiff" role="status">{{ t("models.no_new_models_found") }}</p>
      <p v-if="diff && (diff.added > 0 || diff.missing > 0)" class="mdiff" role="status">
        {{ diff.added > 0 ? t("models.found_n_new_models", { n: diff.added }) : "" }}{{ diff.missing > 0 ? t("models.n_configured_models_were_not", { n: diff.missing }) : "" }}
      </p>
      <ModelChoice
        :models="models"
        :picked="picked"
        :vision="vision"
        :def="def"
        :facts="facts"
        :vision-locked="visionLocked"
        :on-check="checkModel"
        :aliases="aliases"
        :labels="entry.modelShorts"
        :on-alias="(m, value) => (aliases = { ...aliases, [m]: value })"
        :check-disabled="busy !== '' || testing"
        :on-toggle="(m) => toggle(picked, (v) => (picked = v), m)"
        :on-vision="(m) => toggle(vision, (v) => (vision = v), m)"
        :on-default="(m) => (def = m)"
        :on-add="addModel"
        :on-fetch="listable ? refetch : undefined"
        :fetching="refreshing"
        :fetch-disabled="busy !== '' || testing"
        :fetch-fail="refreshFail"
      />
    </div>

    <!-- Folded, and worth folding: no probe can answer these, and most
         endpoints need none of them. -->
    <EndpointSwitches
      v-if="wireSwitches && hasEndpointSwitches(entry)"
      :entry="entry" :label="label" :disabled="saving" :shape="shapeWhy"
      :search="search" :sends-thinking="think !== 'none'" :continuation="carry"
      :on-search="(on) => (search = on)" :on-thinking="setThinking" :on-continuation="(mode) => (carry = mode)"
    />

    <details class="addp-options" :open="more" @toggle="setupMore">
      <summary data-action="provider.draft" data-value="compat">
        <span class="tx">
          <strong>{{ t("models.thinking_and_reasoning_levels") }}</strong>
          <small>{{ t("models.plus_extra_headers_and_body") }}</small>
        </span>
        <span class="summary-value">
          {{ summary || t("common.optional") }}
        </span>
      </summary>
      <div v-if="more" class="fields compat addp-options-body">
        <label class="grow full">
          <span>{{ t("models.thinking_parameter") }}</span>
          <select ref="reasoning" :value="think" @change="think = ($event.target as HTMLSelectElement).value">
            <option v-for="[value, text] in THINKING" :key="value" :value="value">
              {{ t(text) }}
            </option>
          </select>
          <i class="tip">
            {{ t("models.how_this_endpoint_expresses_thinking") }}
          </i>
          <EffortShape v-if="!(wireSwitches && entry.canSetThinking)" class="tip"
            :field="think === (entry.reasoningProtocol ?? '') || kind !== entry.kind ? effortField : undefined" :level="levels[0]" />
        </label>
        <label class="grow">
          <span>{{ t("models.reasoning_levels") }}</span>
          <input
            data-action="provider.draft"
            data-value="effort-levels"
            :value="levelText"
            spellcheck="false"
            placeholder="low, medium, high"
            :disabled="levelsDormant"
            @input="levelText = ($event.target as HTMLInputElement).value"
          />
          <i class="tip">
            {{ levelsDormant
              ? t("models.this_thinking_parameter_does_not")
              : effortField
                ? t("models.the_values_this_endpoint_accepts", { field: effortField })
                : t("models.the_values_this_endpoint_accepts_2") }}
          </i>
        </label>
        <label class="grow">
          <span>{{ t("models.default_level") }}</span>
          <select data-action="provider.draft" data-value="default-effort" :value="levels.includes(defEffort) ? defEffort : ''" :disabled="levelsDormant || levels.length === 0"
            @change="defEffort = ($event.target as HTMLSelectElement).value">
            <option value="">{{ t("models.the_first_level") }}</option>
            <option v-for="l in levels" :key="l" :value="l">{{ l }}</option>
          </select>
          <i class="tip">{{ t("models.the_level_used_when_reasoning") }}</i>
        </label>
        <ModelEfforts
          v-if="picked.length > 1 || picked.some((m) => ownEfforts[m])"
          :models="picked"
          :value="ownEfforts"
          :on-change="(next) => (ownEfforts = next)"
          :inherited="inheritedFor"
          :protocols="entry.modelProtocols ?? {}"
          :conn-protocol="think"
        />
        <label class="grow">
          <span>{{ t("models.no_answer_timeout") }}</span>
          <span class="unit-field">
            <input
              :aria-label="t('models.no_answer_timeout')"
              data-action="provider.draft"
              data-value="idle-timeout"
              inputmode="numeric"
              :value="idleText"
              :placeholder="entry.idleTimeoutDefault ? t('models.default_n', { n: entry.idleTimeoutDefault }) : t('common.default')"
              :aria-invalid="!idle.ok || undefined"
              @input="idleText = ($event.target as HTMLInputElement).value"
            />
            <i>{{ t("common.s") }}</i>
          </span>
          <i class="tip">
            {{ t("models.if_the_server_sends_nothing") }}
          </i>
        </label>
        <BalanceFields :entry="entry" :url="balUrl" :format="balFormat" :on-url="(v) => (balUrl = v)" :on-format="(v) => (balFormat = v)" />
        <div v-if="!idle.ok" class="why">{{ t("models.enter_a_whole_number_of", { min: IDLE_TIMEOUT_MIN, max: IDLE_TIMEOUT_MAX }) }}</div>
        <label class="grow full">
          <span>{{ t("models.extra_request_headers") }}</span>
          <textarea
            rows="3"
            :value="heads"
            spellcheck="false"
            :placeholder="HEADERS_PLACEHOLDER"
            @input="heads = ($event.target as HTMLTextAreaElement).value"
          />
          <i class="tip">{{ t("models.one_name_value_per_line") }}</i>
        </label>
        <label class="grow full">
          <span>{{ t("models.extra_request_body") }}</span>
          <textarea
            rows="4"
            :value="extra"
            spellcheck="false"
            :placeholder="EXTRA_PLACEHOLDER"
            :aria-invalid="extraBad || undefined"
            @input="extra = ($event.target as HTMLTextAreaElement).value"
          />
          <i class="tip">
            {{ t("models.merged_into_the_top_level") }}
          </i>
        </label>
        <div v-if="extraBad" class="why">{{ t("models.this_is_not_a_valid") }}</div>
      </div>
    </details>

    <div class="acts-bar">
      <div v-if="err" class="find" :data-lvl="err.kind === 'unapplied' ? 'warn' : 'err'" :role="err.kind === 'unapplied' ? 'status' : 'alert'">
        <span v-if="err.kind === 'save'" class="t">{{ t("models.save_failed") }}</span>
        <span class="why">{{ err.text }}</span>
      </div>
      <div class="acts">
        <button class="act" data-action="provider.save" data-primary @click="save" :disabled="!canSave">
          {{ t(saving ? "common.saving" : "common.save") }}
        </button>
        <button class="act" data-action="provider.revert" @click="onRevert" :disabled="!dirty || busy !== '' || testing">{{ t("common.revert") }}</button>
        <span class="acts-state" role="status" :data-state="phase">
          <i class="acts-dot" aria-hidden="true" />
          {{ t(PHASE_TEXT[phase]) }}
        </span>
      </div>
    </div>
  </fieldset>
</template>
