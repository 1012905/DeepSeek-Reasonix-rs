<script setup lang="ts">
import { t } from "../i18n";
import type { MenuItem } from "./Menu.vue";

/** One chip per provider the model menu lists; the chosen ones narrow it. Drawn
 *  only where there is a choice to make, so a single provider shows nothing. */
defineProps<{ headings: MenuItem[]; chosen: readonly string[]; onChange: (next: string[]) => void }>();
</script>

<template>
  <div v-if="headings.length >= 2" class="mchips" role="group" :aria-label="t('models.filter_by_provider')">
    <button
      v-for="h in headings"
      :key="h.value"
      type="button"
      class="mchip"
      data-action="model.group"
      :data-target="h.value"
      :aria-pressed="chosen.includes(h.value)"
      @click="onChange(chosen.includes(h.value) ? chosen.filter((key) => key !== h.value) : [...chosen, h.value])"
    >
      {{ h.label }}
    </button>
    <button v-if="chosen.length > 0" type="button" class="mchip clear" data-action="model.group-clear" @click="onChange([])">
      {{ t("common.clear_2") }}
    </button>
  </div>
</template>
