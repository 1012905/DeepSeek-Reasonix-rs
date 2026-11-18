<script lang="ts">
import { computed, defineComponent, h, ref, watch, type PropType, type VNodeChild } from "vue";
import { t } from "../i18n";
import type { ProviderModelCheck, ProviderModelCheckReason, ProviderModelCheckStatus } from "../port/port";
import StudioIcon from "./StudioIcon.vue";

// Which models a connection offers. A gateway answers with hundreds of names
// that differ by a date suffix, so the list is a search field first: one field
// that filters, and — when nothing on the list carries the name — adds it.

// A list of DOM rows needs a ceiling; only unticked tail rows give way, because
// "what did I pick" is the question this panel exists to answer.
const CAP = 60;

export type ModelOrigin = "configured" | "endpoint" | "manual" | "missing";

export interface ModelFact {
  origin: ModelOrigin;
  status?: ProviderModelCheckStatus;
  reason?: ProviderModelCheckReason;
  httpStatus?: number;
  detail?: string;
  checking?: boolean;
}

export function checkedFact(fact: ModelFact | undefined, origin: ModelOrigin, got: ProviderModelCheck): ModelFact {
  return { ...(fact ?? { origin }), status: got.status, reason: got.reason, httpStatus: got.httpStatus, detail: got.detail };
}

export function clearModelCheckFacts(facts: Record<string, ModelFact>): Record<string, ModelFact> {
  return Object.fromEntries(Object.entries(facts).map(([model, fact]) => [
    model,
    { origin: fact.origin },
  ]));
}

const ORIGIN: Record<ModelOrigin, string> = {
  configured: "composer.configured",
  endpoint: "composer.endpoint_catalog",
  manual: "composer.added_manually",
  missing: "composer.not_returned_this_time",
};

function unavailableReason(reason?: ProviderModelCheckReason): string {
  switch (reason) {
    case "not_found": return "composer.the_model_does_not_exist";
    case "rejected": return "composer.the_endpoint_does_not_accept";
    case "tools": return "composer.answers_chat_but_refuses_tool";
    default: return "composer.currently_unavailable";
  }
}

function checkReason(reason?: ProviderModelCheckReason): string {
  switch (reason) {
    case "auth": return "composer.credentials_were_rejected_availability_is";
    case "rate_limited": return "composer.rate_limited_availability_is_still";
    case "network": return "composer.network_error_availability_is_still";
    case "timeout": return "composer.verification_timed_out_availability_is";
    case "local_timeout": return "composer.the_local_service_did_not";
    case "redirect_refused": return "composer.the_address_redirected_to_another";
    case "rejected": return "composer.request_rejected_availability_is_still";
    default: return "composer.availability_is_still_unknown";
  }
}

function cap(hits: string[], on: Set<string>): [string[], number] {
  if (hits.length <= CAP) return [hits, 0];
  let room = Math.max(CAP - hits.filter((m) => on.has(m)).length, 0);
  const out = hits.filter((m) => on.has(m) || room-- > 0);
  return [out, hits.length - out.length];
}

function checkTally(picked: string[], facts?: Record<string, ModelFact>): string {
  if (!facts || !picked.some((m) => facts[m]?.status || facts[m]?.checking)) return "";
  const count = { available: 0, unavailable: 0, unknown: 0, checking: 0, todo: 0 };
  for (const m of picked) count[facts[m]?.checking ? "checking" : facts[m]?.status ?? "todo"]++;
  return [
    count.available && t("composer.n_available", { n: count.available }),
    count.unavailable && t("composer.n_unavailable", { n: count.unavailable }),
    count.unknown && t("composer.n_undetermined", { n: count.unknown }),
    count.checking && t("composer.n_verifying", { n: count.checking }),
    count.todo && t("composer.n_not_verified", { n: count.todo }),
  ].filter(Boolean).join(" · ");
}

const ModelEvidence = defineComponent({
  name: "ModelEvidence",
  props: { fact: { type: Object as PropType<ModelFact>, required: true } },
  setup(props) {
    const state = computed(() => {
      const f = props.fact;
      if (f.checking) return "common.verifying";
      if (f.status === "available") return "composer.verified_available";
      if (f.status === "unavailable") return unavailableReason(f.reason);
      if (f.status === "unknown") return checkReason(f.reason);
      return "composer.not_verified";
    });
    return () => {
      const f = props.fact;
      const children: VNodeChild[] = [h("span", null, t(ORIGIN[f.origin]))];
      children.push(h("i", { "aria-hidden": "true", "data-state": f.checking ? "checking" : f.status ?? "unverified" }), t(state.value));
      if (!f.checking && f.httpStatus) children.push(h("code", { class: "mhttp" }, `HTTP ${f.httpStatus}`));
      if (!f.checking && f.detail) children.push(h("span", { class: "mdetail" }, f.detail));
      return h("span", { class: "mevidence", role: f.status === "unavailable" && !f.checking ? "alert" : undefined }, children);
    };
  },
});
</script>

<script setup lang="ts">
// Everything on offer: what the endpoint reported, plus anything added here.
const props = defineProps<{
  models: string[];
  aliases?: Record<string, string>;
  onAlias?: (m: string, value: string) => void;
  picked: string[];
  vision: string[];
  // Absent where the flow has no default to set — adding a source takes the
  // first pick, so there is nothing to choose yet.
  onDefault?: (m: string) => void;
  def?: string;
  // Absent where the answer is the endpoint's and not the user's to change.
  onVision?: (m: string) => void;
  // Per row: the kernel refuses image input for this model, so the switch would
  // be a dead control. One endpoint can serve both kinds.
  visionLocked?: (m: string) => boolean;
  // How a model reads with its shared prefix left out; the id stays in the tooltip.
  labels?: Record<string, string>;
  facts?: Record<string, ModelFact>;
  onCheck?: (m: string) => void;
  checkDisabled?: boolean;
  onToggle: (m: string) => void;
  onAdd: (m: string) => void;
  // The same read the section header offers, placed where a missing model is noticed.
  onFetch?: () => void;
  fetching?: boolean;
  fetchDisabled?: boolean;
  fetchFail?: string;
}>();

const q = ref("");
const query = computed(() => q.value.trim().toLowerCase());
const aliasFor = (m: string) => (props.aliases && Object.hasOwn(props.aliases, m) ? props.aliases[m] : "");
const on = computed(() => new Set(props.picked));

// Enabled first, and keyed on the list alone: re-sorting on every tick throws
// the row out from under the pointer that just hit it.
let sortPicked = props.picked;
watch(() => props.models, () => { sortPicked = props.picked; });
const order = computed(() => [...props.models].sort((a, b) => Number(sortPicked.includes(b)) - Number(sortPicked.includes(a))));

const hits = computed(() => (query.value ? order.value.filter((m) => `${m} ${aliasFor(m)}`.toLowerCase().includes(query.value)) : order.value));
const capped = computed(() => cap(hits.value, on.value));
const shown = computed(() => capped.value[0]);
const hidden = computed(() => capped.value[1]);
// Display aliases aid search but cannot reserve another model's wire ID.
const naming = computed(() => q.value.trim() !== "" && !props.models.some((m) => m.toLowerCase() === query.value));

const add = () => {
  props.onAdd(q.value.trim());
  q.value = "";
};

const searchKey = (e: KeyboardEvent) => {
  if (e.key !== "Enter" || !naming.value) return;
  e.preventDefault();
  add();
};

const tally = computed(() => checkTally(props.picked, props.facts));

const visionTitle = (m: string) => props.visionLocked?.(m)
  ? t("composer.this_connection_will_not_send")
  : t(props.vision.includes(m) ? "composer.image_input_enabled" : "composer.image_capability_is_unverified_click");
</script>

<template>
  <p v-if="facts" class="msummary" role="status">{{ tally }}</p>
  <div class="msearch">
    <input
      type="search"
      :value="q"
      spellcheck="false"
      :placeholder="t('composer.search_or_enter_an_exact')"
      :aria-label="t('composer.search_or_add_a_model')"
      @input="q = ($event.target as HTMLInputElement).value"
      @keydown="searchKey"
    />
    <span v-if="query" class="cnt">{{ hits.length }} / {{ models.length }}</span>
    <button v-if="onFetch" class="mfetch" data-action="provider.probe" @click="onFetch()" :disabled="fetchDisabled">
      {{ t(fetching ? "common.reading_from_provider" : "composer.not_listed_read_available_models") }}
    </button>
  </div>
  <div v-if="fetchFail" class="find" data-lvl="err" role="alert">
    <span class="t">{{ t("composer.could_not_read_the_model") }}</span>
    <span class="why">{{ fetchFail }}</span>
    <span class="why">{{ t("composer.you_can_also_type_the") }}</span>
  </div>

  <div class="mrows">
    <div v-for="m in shown" :key="m" class="mline" :data-off="on.has(m) ? undefined : ''"
      :aria-busy="facts?.[m]?.checking || undefined">
      <button class="tick" role="checkbox" :aria-checked="on.has(m)"
        :aria-label="t('composer.enable_name', { name: m })" @click="onToggle(m)">
        <i />
      </button>
      <span class="mid">
        <span class="nm" :title="labels?.[m] ? m : undefined">{{ labels?.[m] ?? m }}</span>
        <input v-if="onAlias" class="malias" :value="aliasFor(m)" spellcheck="false"
          data-action="provider.draft" data-value="model-alias" :data-target="m"
          :aria-label="t('composer.display_alias_for_name', { name: m })"
          :placeholder="t('composer.display_alias_empty_uses_model')"
          :title="t('composer.display_only_the_model_id')"
          :disabled="!on.has(m)" @input="onAlias(m, ($event.target as HTMLInputElement).value)" />
        <ModelEvidence v-if="facts?.[m]" :fact="facts[m]" />
      </span>
      <span class="mops">
        <button v-if="onCheck" class="mverify" data-action="provider.model-check" :data-target="m"
          :disabled="!on.has(m) || checkDisabled || facts?.[m]?.checking"
          :aria-label="t('composer.verify_model_name', { name: m })" @click="onCheck(m)">
          {{ t(facts?.[m]?.checking ? "common.verifying" : facts?.[m]?.status ? "composer.verify_again" : "composer.verify") }}
        </button>
        <button v-if="onVision" class="vtag" :aria-pressed="vision.includes(m)"
          :aria-label="t('composer.image_input_for_name', { name: m })"
          :disabled="!on.has(m) || visionLocked?.(m)"
          :title="visionTitle(m)"
          @click="onVision(m)">
          {{ t(vision.includes(m) ? "common.reads_images" : visionLocked?.(m) ? "composer.no_images" : "composer.images") }}
        </button>
        <span v-else-if="vision.includes(m)" class="vtag" data-flat="">{{ t("common.reads_images") }}</span>
        <button v-if="onDefault" class="dtag" :aria-pressed="def === m" :disabled="!on.has(m)"
          :aria-label="t('composer.make_name_the_default_model', { name: m })"
          @click="onDefault(m)">
          {{ t("common.default") }}
        </button>
      </span>
    </div>
  </div>

  <span v-if="hidden > 0" class="mmore">{{ t("composer.n_more_not_listed_search", { n: hidden }) }}</span>
  <div v-if="shown.length === 0 && !naming" class="empty">{{ t("composer.no_models_match") }}</div>

  <!-- Under the list, never over it: a "create" offer sitting above seven real
       matches is how you add a model when you meant to pick one. -->
  <button v-if="naming" class="mnew" @click="add">
    <span class="k"><StudioIcon name="plus" /></span>
    <span class="tx">
      <span class="lb">{{ t("composer.add_unlisted_model_name", { name: q.trim() }) }}</span>
      <span class="ds">{{ t("composer.saves_the_exact_model_id") }}</span>
    </span>
  </button>
</template>
