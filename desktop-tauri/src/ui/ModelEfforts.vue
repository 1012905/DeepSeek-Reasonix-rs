<script lang="ts">
import { defineComponent, h, ref, computed, type PropType } from "vue";
import { t } from "../i18n";
import type { ModelEffort } from "../port/port";
import { parseEffortLevels } from "./provider_compat";

// The wire vocabulary in depth order, so chips read shallow to deep whatever
// order a declaration was stored in. Levels outside it follow in their own order.
const DEPTH = ["none", "disabled", "minimal", "low", "medium", "high", "xhigh", "max"];

// A protocol with a fixed vocabulary, or none at all, leaves a declared list
// on file with nothing to act on.
export const dormantProtocol = (protocol: string) => protocol === "kimi-k3" || protocol === "none";

function officialNote(from: ModelEffort): string {
  const levels = from.supportedEfforts.join(" · ");
  return from.defaultEffort
    ? t("models.official_levels_levels_default_def", { levels, def: from.defaultEffort })
    : t("models.official_levels_levels", { levels });
}

function seededDefault(from?: ModelEffort | null): string {
  const def = from?.defaultEffort ?? "";
  return from?.supportedEfforts.includes(def) ? def : "";
}

function withDefault(levels: string[], def?: string): ModelEffort {
  return { supportedEfforts: levels, defaultEffort: def && levels.includes(def) ? def : "" };
}

function candidates(own: string[], inherited: string[]): string[] {
  const out = [...DEPTH];
  for (const level of [...inherited, ...own]) if (!out.includes(level)) out.push(level);
  return out;
}

function ordered(levels: string[], offered: string[]): string[] {
  return offered.filter((l) => levels.includes(l));
}

const OwnLevels = defineComponent({
  name: "OwnLevels",
  props: {
    model: { type: String, required: true },
    own: { type: Object as PropType<ModelEffort>, required: true },
    from: { type: null as unknown as PropType<ModelEffort | null>, default: undefined },
    onChange: { type: Function as PropType<(next: ModelEffort) => void>, required: true },
  },
  setup(props) {
    const extra = ref("");
    const levels = computed(() => props.own.supportedEfforts);
    const offered = computed(() => candidates(levels.value, props.from?.supportedEfforts ?? []));
    const toggle = (level: string) => {
      const ls = levels.value;
      const picked = ls.includes(level) ? ls.filter((l) => l !== level) : [...ls, level];
      props.onChange(withDefault(ordered(picked, offered.value), props.own.defaultEffort));
    };
    const addTyped = () => {
      const typed = parseEffortLevels(extra.value);
      if (typed.length === 0) return;
      props.onChange(withDefault([...levels.value, ...typed.filter((l) => !levels.value.includes(l))], props.own.defaultEffort));
      extra.value = "";
    };
    return () => h("div", { class: "me-own" }, [
      h("div", { class: "me-chips" }, [
        ...offered.value.map((level) => h("button", {
          key: level,
          type: "button",
          class: "chip",
          "aria-pressed": levels.value.includes(level),
          "data-action": "provider.draft",
          "data-value": "model-effort-level",
          onClick: () => toggle(level),
        }, level)),
        h("input", {
          class: "me-extra",
          "data-action-change": "provider.draft",
          "data-action-keydown": "provider.draft",
          "data-value": "model-effort-extra",
          value: extra.value,
          spellcheck: false,
          placeholder: t("models.other_level"),
          "aria-label": t("models.another_level_for_model", { model: props.model }),
          onInput: (e: Event) => { extra.value = (e.target as HTMLInputElement).value; },
          onBlur: addTyped,
          onKeydown: (e: KeyboardEvent) => { if (e.key === "Enter") { e.preventDefault(); addTyped(); } },
        }),
      ]),
      h("label", { class: "me-default" }, [
        h("span", null, t("models.default_level")),
        h("select", {
          "aria-label": t("models.default_level_for_model", { model: props.model }),
          value: props.own.defaultEffort ?? "",
          "data-action": "provider.draft",
          "data-value": "model-default-effort",
          disabled: levels.value.length === 0,
          onChange: (e: Event) => props.onChange({ supportedEfforts: levels.value, defaultEffort: (e.target as HTMLSelectElement).value }),
        }, [
          h("option", { value: "" }, levels.value[0] ? t("models.the_first_level_level", { level: levels.value[0] }) : t("models.the_first_level")),
          ...levels.value.map((l) => h("option", { key: l, value: l }, l)),
        ]),
      ]),
      levels.value.length === 0 ? h("i", { class: "tip me-note" }, t("models.with_no_level_picked_the")) : null,
    ]);
  },
});
</script>

<script setup lang="ts">

// One row per enabled model: inherit the connection's levels, or pick the
// model's own. Only a model with its own levels carries an entry in value.
const props = defineProps<{
  models: string[];
  value: Record<string, ModelEffort>;
  onChange: (next: Record<string, ModelEffort>) => void;
  // What each model runs on when it declares nothing: the connection's typed
  // list, else the kernel's answer for the saved config. null is an answer
  // that waits on a connection change not yet saved.
  inherited: (model: string) => ModelEffort | null | undefined;
  protocols: Record<string, string>;
  connProtocol: string;
}>();

const set = (model: string, next: ModelEffort | undefined) => {
  const out = { ...props.value };
  if (next) out[model] = next;
  else delete out[model];
  props.onChange(out);
};

const inheritNote = (model: string): string => {
  const from = props.inherited(model);
  return from === null
    ? t("models.inherits_settled_by_the_connection")
    : from?.official
      ? officialNote(from)
      : from?.supportedEfforts.length
        ? t("models.inherits_levels", { levels: from.supportedEfforts.join(" · ") })
        : t("models.inherits_the_endpoint_declares_no");
};
</script>

<template>
  <div class="grow full model-efforts" role="group" :aria-label="t('models.levels_per_model')">
    <span>{{ t("models.levels_per_model") }}</span>
    <i class="tip">{{ t("models.when_one_connection_serves_several") }}</i>
    <ul class="me-list">
      <li v-for="model in models" :key="model" class="me-row" :data-model="model">
        <span class="me-name" :title="model">{{ model }}</span>
        <select
          :aria-label="t('models.reasoning_levels_for_model', { model })"
          data-action="provider.draft" data-value="model-effort-mode"
          :value="value[model] ? 'own' : 'inherit'"
          :disabled="dormantProtocol(protocols[model] || connProtocol)"
          @change="set(model, ($event.target as HTMLSelectElement).value === 'own'
            ? { supportedEfforts: inherited(model)?.supportedEfforts ?? [], defaultEffort: seededDefault(inherited(model)) }
            : undefined)"
        >
          <option value="inherit">{{ inherited(model)?.official ? t("models.inherit_official_levels") : t("models.inherit_from_connection") }}</option>
          <option value="own">{{ t("models.its_own_levels") }}</option>
        </select>
        <i v-if="dormantProtocol(protocols[model] || connProtocol)" class="tip me-note">{{ t("models.this_models_thinking_parameter_does") }}</i>
        <OwnLevels v-else-if="value[model]" :model="model" :own="value[model]" :from="inherited(model)" :on-change="(next: ModelEffort) => set(model, next)" />
        <i v-else class="tip me-note">{{ inheritNote(model) }}</i>
      </li>
    </ul>
  </div>
</template>