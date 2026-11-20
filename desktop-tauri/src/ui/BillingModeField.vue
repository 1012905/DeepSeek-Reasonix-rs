<script lang="ts">
export const PAYG = "payg";
export const SUBSCRIPTION = "subscription_equivalent";
</script>

<script setup lang="ts">
import { t } from "../i18n";

const props = defineProps<{ value: string; onChange: (v: string) => void }>();

const handleChange = (e: Event) => props.onChange((e.target as HTMLSelectElement).value);
</script>

<template>
  <label class="grow full">
    <span>{{ t("usage.billing") }}</span>
    <select :aria-label="t('usage.billing')" data-action="provider.draft" data-value="billing-mode" :value="value" @change="handleChange">
      <option :value="PAYG">{{ t("usage.pay_as_you_go_default") }}</option>
      <option :value="SUBSCRIPTION">{{ t("usage.subscription_plan_estimated_at_the") }}</option>
    </select>
    <i class="tip">{{ t("usage.a_plans_usage_is_converted") }}</i>
  </label>
</template>