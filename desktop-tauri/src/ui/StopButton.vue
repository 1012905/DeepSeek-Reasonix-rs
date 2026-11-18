<script setup lang="ts">
import { computed } from "vue";
import { Square } from "lucide-vue-next";
import { t } from "../i18n";

const props = defineProps<{ stopping: boolean; onStop: () => void }>();
const label = computed(() => t(props.stopping ? "composer.stopping" : "composer.stop"));
</script>

<template>
  <button
    class="btn stop"
    data-primary="true"
    data-action="session.stop"
    :data-pending="stopping ? '' : undefined"
    :aria-label="label"
    :disabled="stopping"
    :aria-busy="stopping"
    @click="onStop"
  >
    <span class="ic" aria-hidden="true">
      <Square fill="currentColor" :stroke-width="0" />
    </span>
    <span>{{ label }}</span>
  </button>
</template>