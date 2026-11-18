<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { DisplayCurrencyMode, DisplayCurrencySettings } from "../../../port/boundary";
import SettingRow from "../../SettingRow.vue";

const MODES: [DisplayCurrencyMode, string][] = [
  ["auto", "common.auto"],
  ["CNY", "usage.chinese_yuan_cny"],
  ["USD", "usage.us_dollar_usd"],
];

// The choice relabels every cost the interface shows, and it applies live: the
// kernel announces it and the ledgers rebind, with no rebuild.
const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const state = ref<DisplayCurrencySettings | null>(null);
const busy = ref(false);
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.displayCurrency().then((s) => { if (live) state.value = s; }).catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

async function pick(mode: DisplayCurrencyMode): Promise<void> {
  if (!state.value || mode === state.value.mode || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    state.value = await props.port.saveDisplayCurrency(mode);
    props.onChanged();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// Only what cannot be shown in the pinned currency is named. A price the kernel
// could convert is not a caveat, and listing it would read as one.
const unconverted = computed(() => state.value?.unconverted ?? []);
</script>

<template>
  <SettingRow id="currency" :title="t('usage.cost_display_currency')" :desc="t('settings.which_currency_costs_show')">
    <div class="seg" data-text role="radiogroup" :aria-label="t('usage.display_currency')">
      <button
        v-for="[mode, label] in MODES"
        :key="mode"
        role="radio"
        data-action="currency.mode"
        :data-value="mode"
        :aria-checked="state?.mode === mode"
        :disabled="busy || !state"
        @click="pick(mode)"
      >{{ t(label) }}</button>
    </div>
  </SettingRow>
  <p v-if="!state" class="why">{{ t("usage.could_not_read_the_currency") }}</p>
  <p v-else-if="state.mode !== 'auto' && unconverted.length > 0" class="why">
    {{ t("usage.these_models_prices_are_not", { models: unconverted.map((u) => `${u.model}（${u.currency}）`).join("、") }) }}
  </p>
  <p v-if="error" class="why" role="alert">{{ error }}</p>
</template>
