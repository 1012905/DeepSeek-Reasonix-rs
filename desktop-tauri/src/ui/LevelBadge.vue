<script lang="ts">
import { Fragment, h, type Component, type VNode } from "vue";

const ground = (): VNode => h("path", { d: "M4.5 20.5h15" });

function petal(angle: number): VNode {
  return h("ellipse", { key: angle, cx: "12", cy: "4.5", rx: "2.1", ry: "2.7", transform: `rotate(${angle} 12 9.5)`, fill: "currentColor", stroke: "none" });
}

export const LEVEL_GLYPHS: readonly Component[] = [
  () => h(Fragment, null, [
    h("ellipse", { cx: "12", cy: "13.2", rx: "3.9", ry: "5.9", transform: "rotate(24 12 13.2)", fill: "currentColor", stroke: "none" }),
    ground(),
  ]),
  () => h(Fragment, null, [
    h("path", { d: "M12 20.5V11" }),
    h("path", { d: "M12 11c0-4 3-6.5 7-6.5 0 4-3 6.5-7 6.5z", fill: "currentColor" }),
    ground(),
  ]),
  () => h(Fragment, null, [
    h("path", { d: "M12 20.5V9" }),
    h("path", { d: "M12 13.2c0-3.6 2.5-6.2 6.7-6.2 0 3.6-2.5 6.2-6.7 6.2z", fill: "currentColor" }),
    h("path", { d: "M12 11.6c0-3.2-2.3-5.6-6-5.6 0 3.2 2.3 5.6 6 5.6z", fill: "currentColor" }),
    ground(),
  ]),
  () => h(Fragment, null, [
    h("path", { d: "M12 20.5V10.5" }),
    h("circle", { cx: "8.4", cy: "9.4", r: "3.5", fill: "currentColor", stroke: "none" }),
    h("circle", { cx: "15.6", cy: "9.4", r: "3.5", fill: "currentColor", stroke: "none" }),
    h("circle", { cx: "12", cy: "5.8", r: "3.5", fill: "currentColor", stroke: "none" }),
    ground(),
  ]),
  () => h(Fragment, null, [
    ...[0, 72, 144, 216, 288].map(petal),
    h("circle", { cx: "12", cy: "9.5", r: "1.5", fill: "currentColor", stroke: "none" }),
    h("path", { d: "M12 17.4v3.1" }),
    ground(),
  ]),
  () => h(Fragment, null, [
    h("circle", { cx: "6.8", cy: "10.8", r: "4.4", fill: "currentColor", stroke: "none" }),
    h("circle", { cx: "17.2", cy: "10.8", r: "4.4", fill: "currentColor", stroke: "none" }),
    h("circle", { cx: "12", cy: "7.2", r: "5.2", fill: "currentColor", stroke: "none" }),
    h("path", { d: "M12 21V12.5", "stroke-width": "2.2" }),
  ]),
  () => h(Fragment, null, [
    h("path", { d: "M12 3l5 10H7z", fill: "currentColor" }),
    h("path", { d: "M5 8.8l3.4 6.8H1.6z", fill: "currentColor" }),
    h("path", { d: "M19 8.8l3.4 6.8h-6.8z", fill: "currentColor" }),
    h("path", { d: "M12 13v7.5M5 15.6v4.9M19 15.6v4.9" }),
    ground(),
  ]),
];

const LOCKED: Component = () => h(Fragment, null, [
  h("circle", { cx: "12", cy: "12", r: "9.4", "stroke-dasharray": "2.3 2.7" }),
  h("rect", { x: "8", y: "11.2", width: "8", height: "6.2", rx: "1.4", fill: "currentColor", stroke: "none" }),
  h("path", { d: "M9.8 11.2V9.4a2.2 2.2 0 0 1 4.4 0v1.8" }),
]);
</script>

<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  level: number;
  locked?: boolean;
  size?: number;
  // Set where the level's name is printed beside the badge; otherwise label names it.
  decorative?: boolean;
  label?: string;
}>(), { locked: false, size: 24, decorative: false, label: undefined });

const glyph = computed(() => LEVEL_GLYPHS[Math.min(Math.max(Number.isInteger(props.level) ? props.level : 0, 0), LEVEL_GLYPHS.length - 1)]);
const named = computed(() => !props.decorative && props.label !== undefined);
</script>

<template>
  <svg
    class="lvl-badge"
    viewBox="0 0 24 24"
    :width="size"
    :height="size"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    :data-level="level"
    :data-locked="locked ? '' : undefined"
    :role="named ? 'img' : undefined"
    :aria-label="named ? label : undefined"
    :aria-hidden="named ? undefined : true"
    focusable="false"
  >
    <component :is="locked ? LOCKED : glyph" />
  </svg>
</template>