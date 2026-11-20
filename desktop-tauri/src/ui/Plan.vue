<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { currentStep, stepDone, stepLabel, type PlanStep } from "../state/session";
import { useMarker } from "./marker";
import { stepIndent, stepKeys } from "./PlanSteps.vue";

const props = withDefaults(defineProps<{
  steps: PlanStep[];
  shownElsewhere?: boolean;
  paused?: boolean;
}>(), {
  shownElsewhere: false,
  paused: false,
});

const now = computed(() => currentStep(props.steps));
const done = computed(() => props.steps.filter(stepDone).length);
// No plan is not an empty plan, and a list the task tab already draws at full
// width is the reading of the same thing twice.
const shown = computed(() => props.steps.length > 0 && !props.shownElsewhere);
const keys = computed(() => stepKeys(props.steps));
const pct = computed(() => (props.steps.length === 0 ? 0 : Math.round((done.value / props.steps.length) * 100)));

const wrap = ref<HTMLElement | null>(null);
const mark = useMarker(wrap, ".s[data-now]", "y", [() => props.steps, now]);
const cursor = computed(() => (mark.value ? { y: mark.value.at + 6, h: Math.max(0, mark.value.len - 11) } : null));

const markLabel = (st: PlanStep, i: number) => {
  if (stepDone(st)) return t("common.done_2");
  return i === now.value ? t("plan.step.now") : t("plan.step.todo");
};

const rowStyle = (st: PlanStep, i: number) => {
  const px = stepIndent(st);
  return {
    animationDelay: `${Math.min(i, 6) * 34}ms`,
    marginInlineStart: px === undefined ? undefined : `${px}px`,
  };
};
</script>

<template>
  <div v-if="shown" class="block" data-b="plan">
    <div class="lbl">{{ t("common.plan") }}<span class="c">{{ done }} / {{ steps.length }}</span></div>
    <div class="prog2">
      <span class="track">
        <i :style="{ width: `${pct}%` }" />
      </span>
    </div>
    <div ref="wrap" class="planwrap">
      <i v-if="cursor" class="cursor" :style="{ height: `${cursor.h}px`, transform: `translateY(${cursor.y}px)` }" />
      <div class="plan" :data-paused="paused ? '' : undefined">
        <div
          v-for="(st, i) in steps"
          :key="keys[i]"
          class="s"
          :data-done="stepDone(st) ? '' : undefined"
          :data-now="i === now ? '' : undefined"
          :style="rowStyle(st, i)"
        >
          <span class="b" :aria-label="markLabel(st, i)" />
          <span class="t">
            <span class="ln">{{ stepLabel(st) }}</span>
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
