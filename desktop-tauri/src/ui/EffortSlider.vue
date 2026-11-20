<script setup lang="ts">
import { computed, ref, watchEffect } from "vue";
import type { MenuItem } from "./Menu.vue";
import RMark from "./RMark.vue";

/** The reasoning levels as one slider. It moves continuously under the finger
 *  and settles on the nearest level when let go; moving it only previews, and
 *  letting go chooses, so dragging across four levels asks the model once. */
const props = defineProps<{
  name: string;
  model?: string;
  levels: MenuItem[];
  current: string;
  onPick: (value: string) => void;
}>();

const last = computed(() => props.levels.length - 1);
const clamp = (n: number) => Math.min(last.value, Math.max(0, n));
const at = computed(() => Math.max(0, props.levels.findIndex((level) => level.value === props.current)));
const value = ref<number | null>(null);
const held = ref(false);
const pos = computed(() => value.value ?? at.value);
const near = computed(() => clamp(Math.round(pos.value)));

// Once the level it settled on is the current one, the position is the
// current one's again, and nothing is left to remember.
watchEffect(() => {
  if (!held.value && value.value !== null && clamp(Math.round(value.value)) === at.value) value.value = null;
});

const release = () => {
  held.value = false;
  if (value.value === null) return;
  const to = clamp(Math.round(value.value));
  value.value = to;
  if (to !== at.value) props.onPick(props.levels[to].value);
};
const step = (event: KeyboardEvent) => {
  const to = event.key === "ArrowRight" || event.key === "ArrowUp" ? near.value + 1
    : event.key === "ArrowLeft" || event.key === "ArrowDown" ? near.value - 1
    : event.key === "Home" ? 0
    : event.key === "End" ? last.value
    : null;
  if (to === null) return;
  event.preventDefault();
  value.value = clamp(to);
};
const onInput = (event: Event) => { value.value = Number((event.target as HTMLInputElement).value); };
</script>

<template>
  <div class="effort-slider">
    <div class="es-head">
      <span class="es-name">{{ levels[near].label }}</span>
      <span v-if="model" class="es-model">{{ model }}</span>
    </div>
    <div class="es-track" :data-drag="held ? '' : undefined" :style="{ '--p': pos / last }">
      <span class="es-line" aria-hidden="true" />
      <span class="es-fill" aria-hidden="true" />
      <i v-for="(level, i) in levels" :key="level.value" class="es-tick" aria-hidden="true" :data-on="i <= near ? '' : undefined" :style="{ '--k': i / last }" />
      <span class="es-thumb" aria-hidden="true"><RMark class="es-r" /></span>
      <input
        type="range" min="0" :max="last" step="any" :value="pos"
        :aria-label="name" :aria-valuetext="levels[near].label"
        data-action-change="reasoning.effort-preview"
        data-action-pointerdown="reasoning.effort-preview"
        data-action-pointerup="reasoning.effort"
        data-action-pointercancel="reasoning.effort"
        data-action-keydown="reasoning.effort-preview"
        data-action-keyup="reasoning.effort"
        @input="onInput"
        @pointerdown="held = true"
        @pointerup="release" @pointercancel="release"
        @keydown="step" @keyup="release" @blur="release"
      />
    </div>
    <div class="es-marks" aria-hidden="true">
      <span v-for="(level, i) in levels" :key="level.value" class="es-mark" :data-on="i === near ? '' : undefined" :style="{ '--k': i / last }">{{ level.label }}</span>
    </div>
  </div>
</template>
