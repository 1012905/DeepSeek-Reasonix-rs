<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { ChartSpec } from "./spec";
import { cellText } from "./format";

const props = defineProps<{ spec: ChartSpec; id: string }>();

const SHOWN = 200;
const open = ref(false);
const rows = computed(() => props.spec.data.rows.slice(0, SHOWN));
const rest = computed(() => props.spec.data.rows.length - rows.value.length);
</script>

<template>
  <div class="chart-data">
    <button type="button" class="chart-toggle" data-action="chart.data" :aria-expanded="open" :aria-controls="id" @click="open = !open">{{ open ? t("chat.hide_data") : t("chat.view_data") }}</button>
    <div v-if="open" class="chart-table" :id="id" :tabindex="0" role="region" :aria-label="t('chat.chart_data_title', { title: spec.title })">
      <table>
        <thead>
          <tr><th v-for="c in spec.data.columns" :key="c.name" scope="col" :data-num="c.type === 'number' || undefined">{{ c.name }}</th></tr>
        </thead>
        <tbody>
          <tr v-for="(r, i) in rows" :key="i"><td v-for="(v, c) in r" :key="c" :data-num="spec.data.columns[c].type === 'number' || undefined">{{ cellText(v) }}</td></tr>
        </tbody>
      </table>
      <div v-if="rest > 0" class="chart-more">{{ t("chat.and_n_more_rows", { n: rest }) }}</div>
    </div>
  </div>
</template>