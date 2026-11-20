<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import { useUpdateAvailable } from "../state/updatewatch";

const version = useUpdateAvailable();
const says = computed(() => t("about.version_v_is_available_see", { v: version.value }));
</script>

<template>
  <template v-if="version">
    <i class="upd-dot" aria-hidden="true" :title="says" />
    <span class="sr-only">{{ says }}</span>
  </template>
</template>