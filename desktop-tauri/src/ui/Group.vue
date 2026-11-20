<script lang="ts">
import { defineComponent, h, type VNodeChild } from "vue";
import { t } from "../i18n";
import { SETTING_AT, type SettingScope, type ApplySemantics } from "./prefsnav";

export const SCOPE_SAID: Record<SettingScope, string> = {
  session: "common.this_session",
  model: "settings.this_model",
  workspace: "settings.this_workspace",
  machine: "session.this_machine",
  chosen: "settings.you_choose_where",
};
const APPLY_SAID: Record<Exclude<ApplySemantics, "none">, string> = {
  immediate: "look.in_effect_now",
  "runtime-rebuild": "settings.rebuilds_the_runtime",
  restart: "settings.takes_effect_after_a_restart",
  "on-open": "settings.applies_to_new_sessions",
  "next-turn": "settings.applies_from_the_next_turn",
  "next-rebuild": "settings.applies_at_the_next_session",
};
const isUsual = (entry: { scope: SettingScope; apply: string }) =>
  entry.scope === "machine" && (entry.apply === "immediate" || entry.apply === "none");

export const ApplyNote = defineComponent({
  props: { id: { type: String, required: true } },
  setup(props) {
    return () => {
      const entry = SETTING_AT(props.id);
      if (!entry) return null;
      const apply = entry.apply === "none" ? null : t(APPLY_SAID[entry.apply]);
      return h("p", {
        class: "apply", "data-apply": entry.apply, "data-scope": entry.scope,
        "data-usual": isUsual(entry) ? "" : undefined,
      }, [t(SCOPE_SAID[entry.scope]), apply].filter(Boolean).join(" · "));
    };
  },
});
export const ScopeTag = defineComponent({
  props: { id: { type: String, required: true } },
  setup(props) {
    return () => {
      const entry = SETTING_AT(props.id);
      if (!entry || isUsual(entry)) return null;
      const said = [
        entry.scope === "machine" ? null : t(SCOPE_SAID[entry.scope]),
        entry.apply === "immediate" || entry.apply === "none" ? null : t(APPLY_SAID[entry.apply]),
      ];
      return h("span", { class: "scope-tag" }, said.filter(Boolean).join(" · "));
    };
  },
});
const Content = defineComponent({
  props: ["value"],
  setup(props) { return () => props.value as VNodeChild; },
});
</script>

<script setup lang="ts">
defineProps<{
  id: string;
  title: string;
  hint?: string;
  now?: string;
  action?: VNodeChild;
  inline?: boolean;
}>();
</script>

<template>
  <section class="grp" :id="`set-${id}`" :data-setting="id" :data-inline="inline ? '' : undefined">
    <div class="grp-hd">
      <h4>{{ title }}</h4>
      <ScopeTag :id="id" />
      <span v-if="now" class="now">{{ now }}</span>
      <slot name="action"><Content :value="action" /></slot>
    </div>
    <p v-if="hint" class="hint" :title="hint">{{ hint }}</p>
    <div v-if="$slots.default" class="grp-items"><slot /></div>
    <ApplyNote :id="id" />
  </section>
</template>
