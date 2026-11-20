<script lang="ts">
export const EXAMPLE_LEVEL = "high";
</script>

<script setup lang="ts">
import { t } from "../i18n";
import { effortExample } from "./provider_compat";

withDefaults(defineProps<{ field?: string; level?: string; className?: string }>(), { className: "tip" });
</script>

<template>
  <i v-if="field" :class="className" :data-effort-field="field">{{ t("composer.this_interface_type_sends_the", { field, example: effortExample(field, level || EXAMPLE_LEVEL) }) }}</i>
</template>