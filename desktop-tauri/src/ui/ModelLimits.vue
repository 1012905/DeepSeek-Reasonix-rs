<script lang="ts">
import type { ModelLimit } from "../port/port";

export interface LimitText {
  win: string;
  out: string;
}

// Long ids often differ only at the end, so the middle is what gives way.
export const shortId = (id: string) => (id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-7)}` : id);

export const digits = (s: string) => s.replace(/\D/g, "");

export function limitTextOf(own: ModelLimit | undefined): LimitText {
  return { win: own?.contextWindow ? String(own.contextWindow) : "", out: own?.maxOutputTokens ? String(own.maxOutputTokens) : "" };
}

// A blank field inherits and is never sent as a value. A negative output cap
// stored by hand survives an untouched field, because digits() cannot type one.
export function limitsToSend(picked: string[], text: Record<string, LimitText>, stored: Record<string, ModelLimit>): Record<string, ModelLimit> {
  const out: Record<string, ModelLimit> = {};
  for (const model of picked) {
    const cur = text[model];
    if (!cur) continue;
    const was = stored[model];
    const outText = cur.out === String(was?.maxOutputTokens) ? was.maxOutputTokens : Number(cur.out) || 0;
    const winN = Number(cur.win) || 0;
    if (winN || outText) out[model] = { ...(winN ? { contextWindow: winN } : {}), ...(outText ? { maxOutputTokens: outText } : {}) };
  }
  return out;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";

const props = defineProps<{
  models: string[];
  // How a model reads when its shared prefix is left out; absent draws it as it is.
  labels?: Record<string, string>;
  value: Record<string, LimitText>;
  onChange: (next: Record<string, LimitText>) => void;
  inherited: (model: string) => ModelLimit | null | undefined;
}>();

const set = (model: string, patch: Partial<LimitText>) =>
  props.onChange({ ...props.value, [model]: { ...(props.value[model] ?? { win: "", out: "" }), ...patch } });

const rows = computed(() => props.models.map((model) => {
  const cur = props.value[model] ?? { win: "", out: "" };
  const from = props.inherited(model);
  const own = cur.win !== "" || cur.out !== "";
  return { model, cur, from, own };
}));
</script>

<template>
  <div class="ml-group" role="group" :aria-label="t('models.limits_per_model')">
    <div class="ml-head">
      <strong>{{ t("models.limits_per_model") }}</strong>
      <span>{{ t("models.a_blank_field_follows_the") }}</span>
    </div>
    <ul class="me-list ml-list">
      <li v-for="r in rows" :key="r.model" class="ml-row" :data-model="r.model" :data-own="r.own || undefined">
        <span class="me-name" :title="r.model">{{ labels?.[r.model] ?? shortId(r.model) }}</span>
        <span class="unit-field">
          <input
            data-action="provider.draft" data-value="model-window"
            :aria-label="t('models.context_window_for_model', { model: r.model })" inputmode="numeric" :value="r.cur.win"
            :placeholder="r.from?.contextWindow ? t('models.inherits_n', { n: r.from.contextWindow.toLocaleString('en-US') }) : t('models.inherits_not_declared')"
            @input="set(r.model, { win: digits(($event.target as HTMLInputElement).value) })"
          /><i>{{ t("common.window") }}</i>
        </span>
        <span class="unit-field">
          <input
            data-action="provider.draft" data-value="model-max-output"
            :aria-label="t('models.max_output_for_model', { model: r.model })" inputmode="numeric" :value="r.cur.out"
            :placeholder="r.from?.maxOutputTokens && r.from.maxOutputTokens > 0 ? t('models.inherits_n', { n: r.from.maxOutputTokens.toLocaleString('en-US') }) : t('models.inherits_automatic')"
            @input="set(r.model, { out: digits(($event.target as HTMLInputElement).value) })"
          /><i>{{ t("common.output") }}</i>
        </span>
      </li>
    </ul>
  </div>
</template>