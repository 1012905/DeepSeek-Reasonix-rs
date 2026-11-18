<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, ModelEntry } from "../../../port/port";
import { groupVendors, activeKind, contextLabel, type Vendor } from "../../Models";
import { orderAccounts, useProviderOrder } from "../../../state/providerorder";
import { useModelCatalog } from "../../useModelCatalog";
import { useRoles } from "../../useRoles";
import { protocols } from "./core-protocol";

// Which model does which job. One row per job: what it is for, which model runs
// it, and which service that model is reached through — read off the same
// catalogue and the same account grouping the services page lists, because two
// spellings of one account is how a picker names a service nobody configured.
const props = defineProps<{
  port: AgentPort;
  home: AgentPort;
  modelRef?: string;
  effort?: string;
  onChanged: () => void;
}>();

type Answers = "chat" | "decision" | "search" | "vision";

// Decision is the one job that cannot follow the main model: it asks a question
// set a chat model has no answer for, so its row offers decision sources only
// and says so when there are none.
const ROLES: readonly (readonly [string, string, string, Answers])[] = [
  ["planner", "common.plan", "settings.drafts_a_plan_read_only", "chat"],
  ["subagent", "common.subagents", "usage.work_sent_out", "chat"],
  ["vision", "models.vision", "usage.reads_images_the_main_model", "chat"],
  ["guardian", "models.review", "usage.audits_this_turn_independently", "chat"],
  ["title", "session.auto_name", "usage.generate_short_conversation_titles", "chat"],
  ["decision", "models.decision", "usage.the_backend_system_one_asks", "decision"],
] as const;

const SEARCH_WHY: Record<string, string> = {
  bad_ref: "usage.the_search_model_must_be",
  not_added: "usage.the_search_models_connection_has",
  model_removed: "usage.the_search_model_is_no",
  unsupported: "usage.the_search_models_protocol_has",
  no_credentials: "usage.the_search_models_connection_has_2",
};

const models = ref<ModelEntry[]>([]);
const loaded = ref(false);
const { roles, overrides, loadRoles } = useRoles(() => props.port);
const busy = ref("");
const failed = ref("");
const order = useProviderOrder();

const loadModels = useModelCatalog(() => props.port, () => props.home, (list) => {
  models.value = list;
  loaded.value = true;
});

const load = () => {
  loadModels();
  loadRoles();
};

onMounted(load);
watch(() => [props.port, props.home], load);

const vendors = computed(() => orderAccounts(groupVendors(models.value), order.value));
const shown = (m: ModelEntry) => m.modelAlias?.trim() || m.modelShort || m.model;
const answersOf = (m: ModelEntry): Answers => (m.answers === "decision" ? "decision" : "chat");

// Only what the config or the catalog declares: an inferred "reads images"
// sends the reader to a request the endpoint rejects.
const answersMatch = (m: ModelEntry, answers: Answers) =>
  answers === "search" ? m.webSearch === true
    : answers === "vision" ? m.vision === true && answersOf(m) === "chat"
      : answersOf(m) === answers;

const byRef = (ref?: string) => models.value.find((m) => m.ref === ref);
const serviceOf = (ref?: string) =>
  vendors.value.find((v) => Object.values(v.byKind).some((list) => list.some((m) => m.ref === ref)))?.label ?? "";

const traits = (m?: ModelEntry): string => {
  if (!m) return "";
  const ctx = contextLabel(m.contextWindow);
  return [
    m.vision ? t("common.reads_images") : "",
    m.efforts && m.efforts.length > 1 ? t("usage.adjustable_reasoning") : "",
    ctx ? t("usage.size_context", { size: ctx }) : "",
  ].filter(Boolean).join(" · ");
};

// A pick that moves the running session goes through setModel; a pick that only
// decides which models are listed must not.
const run = async (what: string, fn: () => Promise<unknown>) => {
  busy.value = what;
  failed.value = "";
  try {
    await fn();
    props.onChanged();
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const pickMain = (ref: string) =>
  void run(`main:${ref}`, async () => {
    await props.port.setModel(ref);
    await props.home.setDefaultModel(ref);
    loadModels();
  });

const pickEffort = (level: string) => void run(`effort:${level}`, () => props.port.setEffort(level));

const pickRole = (role: string, ref: string, effort = "auto") =>
  void run(`role:${role}`, async () => {
    await props.port.setRole(role, ref, effort);
    loadRoles();
  });

const clearOverride = (role: string, key: string) =>
  void run(`role:${role}`, async () => {
    await props.port.clearRoleOverride(role, key);
    loadRoles();
  });

// The account whose door is being shown, for the service column of a row that
// follows the main model.
const mainRef = computed(() => models.value.find((m) => m.default)?.ref || props.modelRef || "");
const mainModel = computed(() => byRef(props.modelRef));

// What an attachment reaches: the vision role if assigned, else the sub-agent
// it would be handed to, else the main model.
const assignments = computed(() => roles.value?.roles);
const visionRef = computed(() => assignments.value?.vision || assignments.value?.subagent || props.modelRef || "");
const visionModel = computed(() => byRef(visionRef.value));

const effortLevels = (m?: ModelEntry) => [...new Set(["auto", ...(m?.efforts ?? [])])];
const effortSupported = (m?: ModelEntry) => !!m?.efforts?.some((level) => level !== "auto");

// Options for one row's model picker: the account offers the models of the door
// it is showing, and a model already chosen stays listed even when it is not.
const choicesFor = (answers: Answers, value: string): { vendor: Vendor; rows: ModelEntry[] }[] =>
  vendors.value
    .map((vendor) => {
      const kind = protocols.value[vendor.key] ?? activeKind(vendor, props.modelRef);
      const offered = (answers === "search" ? Object.values(vendor.byKind).flat() : vendor.byKind[kind] ?? [])
        .filter((m) => answersMatch(m, answers) && !(answers === "search" && m.notAdded));
      const held = Object.values(vendor.byKind).flat()
        .find((m) => m.ref === value && answersMatch(m, answers) && !offered.includes(m));
      return { vendor, rows: held ? [held, ...offered] : offered };
    })
    .filter((group) => group.rows.length > 0);

const roleRow = (key: string, answers: Answers) => {
  const set = roles.value?.roles[key] ?? "";
  const inherited = answers === "chat" && key !== "title" ? mainRef.value : "";
  return { set, inherited, model: byRef(set || inherited) };
};
</script>

<template>
  <div v-if="!loaded" class="empty">{{ t("common.loading") }}</div>
  <div v-else-if="models.length === 0" class="empty">{{ t("usage.could_not_read_the_model") }}</div>
  <template v-else>
    <div class="usage" role="table" :aria-label="t('usage.model_uses')">
      <div class="usage-row usage-hd" role="row">
        <span role="columnheader">{{ t("usage.use") }}</span>
        <span role="columnheader">{{ t("usage.model") }}</span>
        <span role="columnheader">{{ t("usage.effort") }}</span>
        <span role="columnheader">{{ t("usage.connections") }}</span>
      </div>

      <!-- The default model: this conversation and most of what it does. -->
      <div class="usage-row" role="row">
        <span class="usage-job" role="rowheader">
          <b>{{ t("common.default_model") }}</b>
          <small>{{ t("usage.this_conversation_and_most_tasks") }}</small>
        </span>
        <span role="cell">
          <select
            class="usage-pick" :aria-label="t('common.default_model')" data-action="model.select"
            :value="mainRef" :disabled="busy !== ''"
            @change="pickMain(($event.target as HTMLSelectElement).value)"
          >
            <optgroup v-for="group in choicesFor('chat', mainRef)" :key="group.vendor.key" :label="group.vendor.label">
              <option v-for="m in group.rows" :key="m.ref" :value="m.ref">
                {{ m.vision ? `${shown(m)} · ${t("common.reads_images")}` : shown(m) }}
              </option>
            </optgroup>
          </select>
        </span>
        <span class="usage-effort" role="cell">
          <span class="usage-effort-label">{{ t("usage.effort") }}</span>
          <select
            class="usage-pick" :aria-label="t('usage.default_model_reasoning_effort')" data-action="reasoning.effort"
            :value="effortLevels(mainModel).includes(props.effort ?? 'auto') ? (props.effort ?? 'auto') : 'auto'"
            :disabled="busy !== '' || !effortSupported(mainModel)"
            @change="pickEffort(($event.target as HTMLSelectElement).value)"
          >
            <template v-if="!effortSupported(mainModel)">
              <option value="auto">{{ t("usage.not_applicable") }}</option>
            </template>
            <template v-else>
              <option v-for="level in effortLevels(mainModel)" :key="level" :value="level">
                {{ level === "auto" ? t("common.auto") : level }}
              </option>
            </template>
          </select>
        </span>
        <span class="usage-conn" role="cell">
          {{ serviceOf(mainRef) }}
          <small>{{ traits(mainModel) }}</small>
        </span>
      </div>

      <div v-for="[key, name, tag, answers] in ROLES" :key="key" class="usage-row" role="row">
        <span class="usage-job" role="rowheader">
          <b>{{ t(name) }}</b>
          <small>{{ t(tag) }}</small>
        </span>
        <span role="cell">
          <select
            class="usage-pick" :aria-label="t(name)" data-action="roles.model"
            :value="roleRow(key, answers).set"
            :disabled="busy !== '' || (answers === 'decision' && choicesFor(answers, '').length === 0)"
            @change="pickRole(key, ($event.target as HTMLSelectElement).value, 'auto')"
          >
            <option value="">
              {{ t(key === "title" ? "usage.do_not_use"
                : answers === "chat" ? "usage.follow_the_main_model"
                : choicesFor(answers, "").length === 0 ? "usage.no_source_yet" : "usage.do_not_use") }}
            </option>
            <optgroup v-for="group in choicesFor(answers, roleRow(key, answers).set)" :key="group.vendor.key" :label="group.vendor.label">
              <option v-for="m in group.rows" :key="m.ref" :value="m.ref">{{ shown(m) }}</option>
            </optgroup>
          </select>
          <!-- An entry in the user file outranks a role's own row, so the row is
               not what runs and has to say so. -->
          <span v-for="o in overrides[key] ?? []" :key="o.key" class="usage-override" :data-scope="o.scope">
            <span>{{ t("usage.key_is_pinned_to_model", { key: o.key, model: o.model }) }}</span>
            <button
              v-if="o.scope === 'user'" type="button" data-action="roles.override.clear" :disabled="busy !== ''"
              @click="clearOverride(key, o.key)"
            >{{ t("usage.follow_this_choice") }}</button>
            <small v-else>{{ t("usage.set_by_the_project_config") }}</small>
          </span>
        </span>
        <span class="usage-effort" role="cell">
          <span class="usage-effort-label">{{ t("usage.effort") }}</span>
          <select
            v-if="key === 'title'"
            class="usage-pick" :aria-label="t('usage.name_reasoning_effort', { name: t(name) })" data-action="roles.effort"
            value="auto" disabled
          >
            <option value="auto">{{ t("usage.not_applicable") }}</option>
          </select>
          <select
            v-else
            class="usage-pick" :aria-label="t('usage.name_reasoning_effort', { name: t(name) })" data-action="roles.effort"
            :value="effortLevels(roleRow(key, answers).model).includes(roles?.efforts?.[key] ?? 'auto') ? (roles?.efforts?.[key] ?? 'auto') : 'auto'"
            :disabled="busy !== '' || answers === 'decision' || !effortSupported(roleRow(key, answers).model)"
            @change="pickRole(key, roleRow(key, answers).set || mainRef, ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="level in effortSupported(roleRow(key, answers).model) ? effortLevels(roleRow(key, answers).model) : ['auto']" :key="level" :value="level">
              {{ level === "auto" ? (effortSupported(roleRow(key, answers).model) ? t("common.auto") : t("usage.not_applicable")) : level }}
            </option>
          </select>
        </span>
        <span class="usage-conn" role="cell" :data-follow="!roleRow(key, answers).set && answers === 'chat' && key !== 'title' ? '' : undefined">
          <template v-if="roleRow(key, answers).set">{{ serviceOf(roleRow(key, answers).set) }}</template>
          <template v-else-if="key === 'title'">{{ t("usage.use_message_preview") }}</template>
          <template v-else-if="answers === 'chat'">{{ t("usage.follows_the_default") }}</template>
          <template v-else-if="choicesFor(answers, '').length === 0">{{ t("usage.add_a_decision_source_under") }}</template>
        </span>
      </div>

      <!-- Independent search: a role of its own, because the model asked to
           search is not always the model that can. -->
      <div v-if="roles" class="usage-row" role="row">
        <span class="usage-job" role="rowheader">
          <b>{{ t("common.web_search") }}</b>
          <small>{{ t("usage.model_for_independent_search_requests") }}</small>
        </span>
        <span role="cell">
          <select
            class="usage-pick" :aria-label="t('common.web_search')" data-action="roles.model"
            :value="!roles.web_search || roles.web_search.toLowerCase() === 'auto' ? '' : roles.web_search"
            :disabled="busy !== ''"
            @change="pickRole('web_search', ($event.target as HTMLSelectElement).value)"
          >
            <option value="">{{ t("common.pick_one_automatically") }}</option>
            <optgroup
              v-for="group in choicesFor('search', !roles.web_search || roles.web_search.toLowerCase() === 'auto' ? '' : roles.web_search)"
              :key="group.vendor.key" :label="group.vendor.label"
            >
              <option v-for="m in group.rows" :key="m.ref" :value="m.ref">{{ shown(m) }}</option>
            </optgroup>
          </select>
        </span>
        <span class="usage-effort" role="cell">{{ t("usage.not_applicable") }}</span>
        <span class="usage-conn" role="cell">
          <template v-if="roles.web_search && roles.web_search.toLowerCase() !== 'auto'">
            {{ SEARCH_WHY[roles.web_search_reason ?? ""] ? t(SEARCH_WHY[roles.web_search_reason ?? ""]) : serviceOf(roles.web_search) }}
          </template>
          <template v-else-if="roles.web_search_effective">
            {{ t("usage.automatic_the_conversation_models_built", { model: serviceOf(roles.web_search_effective) }) }}
          </template>
          <template v-else>{{ t("usage.the_conversation_model_has_no") }}</template>
          <small v-if="choicesFor('search', '').length === 0">{{ t("usage.no_search_models_available") }}</small>
          <small v-if="roles.web_search_source === 'project'">
            {{ roles.web_search_effective
              ? t("usage.project_override_model_is_used", { model: serviceOf(roles.web_search_effective) })
              : t("usage.the_project_config_overrides_this") }}
          </small>
        </span>
      </div>
    </div>

    <div v-if="!roles" class="empty">{{ t("usage.could_not_read_the_role") }}</div>
    <p v-if="failed" class="note" role="alert">{{ failed }}</p>
    <p class="note">
      {{ visionModel?.vision
        ? t("usage.images_the_main_model_cannot_2", { model: visionModel.model })
        : t("usage.images_the_main_model_cannot") }}
    </p>
  </template>
</template>
