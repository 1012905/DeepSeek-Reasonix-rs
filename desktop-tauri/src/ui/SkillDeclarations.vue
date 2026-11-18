<script setup lang="ts">
import { t } from "../i18n";
import type { SkillEntry } from "../port/port";

defineProps<{ sk: SkillEntry }>();
</script>

<template>
  <details v-if="sk.subagent && (sk.model || sk.effort || sk.allowedTools?.length)" class="skprofile">
    <summary :aria-label="t('ext.subagent_declarations_for_name', { name: sk.name })">{{ t("ext.subagent_declarations") }}</summary>
    <p>{{ t("ext.shows_the_skills_declarations_runtime") }}</p>
    <dl>
      <template v-if="sk.model"><dt>{{ t("ext.declared_model") }}</dt><dd>{{ sk.model }}</dd></template>
      <template v-if="sk.effort"><dt>{{ t("ext.declared_reasoning_effort") }}</dt><dd>{{ sk.effort }}</dd></template>
      <template v-if="sk.allowedTools?.length">
        <dt>{{ t("ext.declared_tools") }}</dt>
        <dd><code v-for="(name, index) in sk.allowedTools" :key="index">{{ name }}</code></dd>
      </template>
    </dl>
  </details>
</template>