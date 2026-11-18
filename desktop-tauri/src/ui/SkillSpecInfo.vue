<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import type { SkillEntry } from "../port/port";

// What the skill says about itself in the Agent Skills format: who may use it
// and what it expects. Shown as written; nothing here changes how it runs.
const props = defineProps<{ sk: SkillEntry }>();
const meta = computed(() => Object.entries(props.sk.metadata ?? {}));
</script>

<template>
  <details v-if="sk.license || sk.compatibility || meta.length > 0" class="skprofile">
    <summary :aria-label="t('ext.skill_information_for_name', { name: sk.name })">{{ t("ext.skill_information") }}</summary>
    <dl>
      <template v-if="sk.license"><dt>{{ t("ext.license") }}</dt><dd>{{ sk.license }}</dd></template>
      <template v-if="sk.compatibility"><dt>{{ t("ext.requirements") }}</dt><dd>{{ sk.compatibility }}</dd></template>
      <template v-for="[key, value] in meta" :key="key"><dt>{{ key }}</dt><dd>{{ value }}</dd></template>
    </dl>
  </details>
</template>