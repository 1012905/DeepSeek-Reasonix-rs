<script setup lang="ts">
import { computed } from "vue";

// A speed curve over the last n seconds. No tick labels: the reader wants the
// shape (climbing, stalling, stuttering); the exact number sits right beside it.
const props = withDefaults(defineProps<{ points: number[]; w?: number; h?: number }>(), { w: 92, h: 18 });

// Height normalizes against the highest speed seen over the whole session, not
// the window's. Otherwise the curve rescales every time the window scrolls and
// unchanging data reads as rising and falling — the chart lying.
let peak = 1;
const geom = computed(() => {
  const pts = props.points;
  if (pts.length < 2) peak = 1;
  peak = Math.max(peak, ...pts, 1);
  if (pts.length < 2) return null;
  const step = props.w / (pts.length - 1);
  const y = (v: number) => props.h - 1.5 - (v / peak) * (props.h - 3);
  const line = pts.map((v, i) => `${(i * step).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const last = pts[pts.length - 1];
  return { line, tipY: y(last) };
});
</script>

<template>
  <svg v-if="geom" class="spark" :viewBox="`0 0 ${w} ${h}`" preserveAspectRatio="none" aria-hidden="true">
    <polygon class="fill" :points="`0,${h} ${geom.line} ${w},${h}`" />
    <polyline class="line" :points="geom.line" vector-effect="non-scaling-stroke" />
    <circle class="tip" :cx="w - 0.8" :cy="geom.tipY" :r="1.7" />
  </svg>
</template>