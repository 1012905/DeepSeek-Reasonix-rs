<script lang="ts">
export interface StackPoint { key: string; axis: string; tip: string; hit: number; miss: number; total: number }
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { niceMax } from "./usagescale";

const props = defineProps<{ points: StackPoint[]; label: (v: number) => string; aria: string; single?: boolean }>();

const W = 860, H = 176, PL = 58, PR = 12, PT = 12, PB = 24, GAP = 2;

const at = ref<number | null>(null);
const max = computed(() => niceMax(Math.max(...props.points.map((p) => p.hit + p.miss), 0)));
const band = computed(() => (W - PL - PR) / Math.max(props.points.length, 1));
const bw = computed(() => Math.max(2, Math.min(18, band.value * 0.62)));
const x = (i: number) => PL + band.value * i + (band.value - bw.value) / 2;
const h = (v: number) => (v / max.value) * (H - PT - PB);
const every = computed(() => Math.max(1, Math.ceil(props.points.length / 5)));
const hover = computed(() => (at.value === null ? null : props.points[at.value]));
const bars = computed(() => props.points.map((p) => {
  const hit = h(p.hit), miss = h(p.miss);
  return { key: p.key, hit, miss, gap: hit > 0 && miss > 0 ? GAP : 0 };
}));

const onPointerMove = (e: PointerEvent) => {
  const box = (e.currentTarget as SVGRectElement).ownerSVGElement!.getBoundingClientRect();
  const i = Math.min(props.points.length - 1, Math.max(0, Math.floor((((e.clientX - box.left) / box.width) * W - PL) / band.value)));
  if (i !== at.value) at.value = i;
};
</script>

<template>
  <div class="uplot ustack" :data-on="hover ? '' : undefined">
    <div v-if="!single" class="ulegend" aria-hidden="true">
      <span><i data-s="hit" />{{ t("usage.input_cache_hit") }}</span>
      <span><i data-s="miss" />{{ t("usage.input_cache_miss") }}</span>
    </div>
    <svg :viewBox="`0 0 ${W} ${H}`" role="img" :aria-label="aria">
      <g class="ugrid">
        <line v-for="v in [0, max / 2, max]" :key="v" :x1="PL" :x2="W - PR" :y1="H - PB - h(v)" :y2="H - PB - h(v)" />
      </g>
      <g class="uaxis">
        <text v-for="v in [0, max / 2, max]" :key="v" :x="PL - 8" :y="H - PB - h(v) + 4" text-anchor="end">{{ label(v) }}</text>
        <template v-for="(p, i) in points" :key="p.key">
          <text v-if="i % every === 0" :x="x(i) + bw / 2" :y="H - 7" text-anchor="middle">{{ p.axis }}</text>
        </template>
      </g>
      <g v-for="(b, i) in bars" :key="b.key" :data-on="at === i ? '' : undefined">
        <rect v-if="b.hit > 0" class="useg" data-s="hit" :x="x(i)" :y="H - PB - b.hit" :width="bw" :height="b.hit" :rx="b.miss > 0 ? 0 : Math.min(4, bw / 2)" />
        <rect v-if="b.miss > 0" class="useg" data-s="miss" :x="x(i)" :y="H - PB - b.hit - b.gap - b.miss" :width="bw" :height="b.miss" :rx="Math.min(4, bw / 2)" />
      </g>
      <rect class="uhit" :x="PL" :y="0" :width="W - PL - PR" :height="H" data-action-pointermove="usage.chart-hover" data-action-pointerleave="usage.chart-hover"
        @pointermove="onPointerMove" @pointerleave="at = null" />
    </svg>
    <div v-if="hover && at !== null" class="utip" :style="{ left: `${((x(at) + bw / 2) / W) * 100}%`, top: `${((H - PB - h(hover.hit + hover.miss)) / H) * 100}%` }">
      <div class="d">{{ hover.tip }}</div>
      <div v-if="single" class="r"><b>{{ label(hover.hit) }}</b></div>
      <template v-else>
        <div class="r"><i data-s="hit" />{{ t("usage.input_cache_hit") }} <b>{{ label(hover.hit) }}</b></div>
        <div class="r"><i data-s="miss" />{{ t("usage.input_cache_miss") }} <b>{{ label(hover.miss) }}</b></div>
        <div class="r">{{ t("usage.provider_reported_total") }} <b>{{ label(hover.total) }}</b></div>
      </template>
    </div>
  </div>
</template>