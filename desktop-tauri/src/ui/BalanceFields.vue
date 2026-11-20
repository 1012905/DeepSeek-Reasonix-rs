<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import type { ProviderEntry } from "../port/port";

const props = defineProps<{
  entry: ProviderEntry;
  url: string;
  format: string;
  onUrl: (v: string) => void;
  onFormat: (v: string) => void;
}>();

const FORMAT_LABEL: Record<string, string> = {
  deepseek: "usage.deepseek_format",
  moonshot: "usage.moonshot_format",
  openrouter: "usage.openrouter_format",
};

// The wallet endpoint and how it answers. The host table that implies a format
// lives in the kernel, so the entry says what its saved address implies and this
// form never guesses from a typed one.
const formats = computed(() => props.entry.balanceFormats ?? []);
const saved = computed(() => props.url.trim() === (props.entry.balanceUrl ?? "").trim());
const implied = computed(() => (saved.value ? props.entry.impliedBalanceFormat ?? "" : ""));
const needs = computed(() => props.url.trim() !== "" && props.format === "" && saved.value && implied.value === "");

const handleUrl = (e: Event) => props.onUrl((e.target as HTMLInputElement).value);
const handleFormat = (e: Event) => props.onFormat((e.target as HTMLSelectElement).value);
</script>

<template>
  <label class="grow full">
    <span>{{ t("usage.balance_address") }}</span>
    <input
      :aria-label="t('usage.balance_address')"
      data-action="provider.draft"
      data-value="balance-url"
      :value="url"
      :spellcheck="false"
      placeholder="https://api.deepseek.com/user/balance"
      @input="handleUrl"
    />
    <i class="tip">{{ t("usage.leave_empty_to_show_no") }}</i>
  </label>
  <label v-if="url.trim() !== ''" class="grow full">
    <span>{{ t("usage.balance_format") }}</span>
    <select :aria-label="t('usage.balance_format')" data-action="provider.draft" data-value="balance-format" :value="format" @change="handleFormat">
      <option value="">{{ implied ? t("usage.from_the_address_format", { format: t(FORMAT_LABEL[implied] ?? implied) }) : t("common.no_selection") }}</option>
      <option v-for="f in formats" :key="f" :value="f">{{ t(FORMAT_LABEL[f] ?? f) }}</option>
    </select>
    <i class="tip">{{ t("usage.a_vendors_own_domain_is") }}</i>
  </label>
  <div v-if="needs" class="why">{{ t("usage.balance_format_not_set_this") }}</div>
</template>