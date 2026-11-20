<script setup lang="ts">
import { computed, type CSSProperties } from "vue";
import { t } from "../../../i18n";
import { pct } from "../../../i18n/format";
import { host } from "../../../port/host";
import type { Appearance as Look } from "../../../port/port";
import { readSizeOf, readSteps, TEXT_STEPS } from "../../look";
import { ZOOM_PRESETS } from "../../zoom";
import { chord } from "../../keys";
import Group from "../../Group.vue";

const props = defineProps<{ look: Look; onLook: (next: Look) => void }>();

const MEASURES: [string, string, string][] = [
  ["standard", "look.reading", "look.keeps_a_line_short_enough"],
  ["full", "look.full_width", "look.prose_follows_the_window_with"],
  ["full-all", "look.full_width_composer_too", "look.prose_and_the_composer_both"],
  ["custom", "look.custom_percentage", "look.set_the_conversation_and_composer"],
];

const set = (patch: Partial<Look>) => props.onLook({ ...props.look, ...patch });

// The kernel announces the ranges; until it has, the sliders are held inert
// rather than given a range of their own that would then move under the thumb.
const zoomRange = computed(() => props.look.zoomRange ?? { min: 0.5, max: 2, step: 0.05 });
const widthRange = computed(() => props.look.widthRange ?? { min: 40, max: 100, step: 1 });

// WebKit's slider track does not know the thumb's value, so the filled share is
// handed in from here.
const at = (v: number, min: number, max: number) =>
  ({ "--at": `${max > min ? Math.round(((v - min) / (max - min)) * 100) : 0}%` }) as CSSProperties;

const zoom = computed(() => props.look.zoom || 1);
const width = computed(() => props.look.width || "standard");
const steps = computed(() => readSteps());
const shell = computed(() => host().inShell());
</script>

<template>
  <Group id="size" :title="t('look.size')" :hint="t('look.interface_scales_spacing_and_controls')">
    <p v-if="shell" class="hint">
      {{ t("look.keyboard_up_larger_down_smaller", { up: chord("+"), down: chord("-"), reset: chord("0") }) }}
    </p>

    <div class="prow">
      <span class="tx">{{ t("look.interface") }}</span>
      <div class="seg" data-text role="group" :aria-label="t('look.interface_size')">
        <button
          v-for="[value, name] in ZOOM_PRESETS"
          :key="value"
          data-action="appearance.zoom"
          :data-value="value"
          :aria-pressed="zoom === value"
          @click="set({ zoom: value })"
        >
          {{ t(name) }}
        </button>
      </div>
    </div>

    <div class="prow">
      <span class="tx">{{ t("look.fine_tune") }}</span>
      <input
        class="slider"
        :style="at(zoom, zoomRange.min, zoomRange.max)"
        type="range"
        :min="zoomRange.min"
        :max="zoomRange.max"
        :step="zoomRange.step"
        :disabled="!look.zoomRange"
        :value="zoom"
        :aria-label="t('look.fine_tune_interface_size')"
        data-action="appearance.zoom"
        @input="set({ zoom: Number(($event.target as HTMLInputElement).value) })"
      />
      <span class="now">{{ pct(zoom) }}</span>
    </div>

    <div class="prow">
      <span class="tx">{{ t("look.interface_text") }}</span>
      <div class="seg" data-text role="group" :aria-label="t('look.interface_text_size')">
        <button
          v-for="[value, name] in TEXT_STEPS"
          :key="value"
          data-action="appearance.text-size"
          :data-value="value"
          :aria-pressed="(look.textSize || 1) === value"
          @click="set({ textSize: value === 1 ? 0 : value })"
        >
          {{ t(name) }}
        </button>
      </div>
    </div>

    <div class="prow">
      <span class="tx">{{ t("common.body_text") }}</span>
      <div class="seg" data-text role="group" :aria-label="t('look.body_text_size')">
        <button
          v-for="[value, name] in steps"
          :key="value"
          data-action="appearance.reading-size"
          :data-value="value"
          :aria-pressed="readSizeOf(look.readSize) === value"
          @click="set({ readSize: value })"
        >
          {{ t(name) }}
        </button>
      </div>
    </div>

    <div class="prow">
      <span class="tx">{{ t("look.line_width") }}</span>
      <div class="seg" data-text role="group" :aria-label="t('look.body_line_width')">
        <button
          v-for="[id, name, why] in MEASURES"
          :key="id"
          data-action="appearance.measure"
          :data-value="id"
          :aria-pressed="width === id"
          :title="t(why)"
          :disabled="id === 'custom' && !look.widthRange"
          @click="set({ width: id })"
        >
          {{ t(name) }}
        </button>
      </div>
    </div>

    <div v-if="look.width === 'custom'" class="prow">
      <span class="tx">{{ t("look.conversation_width") }}</span>
      <input
        class="slider"
        type="range"
        :style="at(look.widthPercent ?? 100, widthRange.min, widthRange.max)"
        :min="widthRange.min"
        :max="widthRange.max"
        :step="widthRange.step"
        :disabled="!look.widthRange"
        :value="look.widthPercent ?? 100"
        :aria-label="t('look.conversation_width')"
        :aria-valuetext="pct((look.widthPercent ?? 100) / 100)"
        data-action="appearance.measure"
        data-value="custom"
        @input="set({ widthPercent: Number(($event.target as HTMLInputElement).value) })"
      />
      <span class="now">{{ pct((look.widthPercent ?? 100) / 100) }}</span>
    </div>

    <p class="hint">
      {{ t(width === "custom" ? "look.custom_percentage_resizes_both_the" : "look.line_width_applies_to_prose") }}
    </p>
  </Group>
</template>
