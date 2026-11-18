<script setup lang="ts">
import { computed, ref, watchEffect } from "vue";
import { t } from "../../i18n";
import { currentStep, stepDone, stepLabel, type Item } from "../../state/session";
import type { PlanAction } from "../../port/session";
import { proposedSteps, usePlanLive } from "../planLive";
import PlanSteps, { stepOrdinal, type StepsState } from "../PlanSteps.vue";
import RMark from "../RMark.vue";

type Phase = "pending" | "settled" | "running" | "waiting" | "stopped" | "done";

const STEPS: Record<Phase, StepsState> = {
  pending: "proposed",
  settled: "proposed",
  running: "moving",
  waiting: "waiting",
  stopped: "still",
  done: "still",
};

function badgeOf(phase: Phase, n: number, total: number, settled: string): string {
  switch (phase) {
    case "pending": return t("plan.badge.pending");
    case "running": return t("plan.badge.step", { n, total });
    case "waiting": return t("plan.badge.waiting");
    case "stopped": return t("plan.badge.stopped");
    case "done": return t("plan.badge.done");
    case "settled": return settled;
  }
}

function settledWords(verdict?: string): [string, string] {
  if (verdict === "exit") return [t("plan.settled.exit"), t("plan.settled.exit_note")];
  if (verdict === "revise" || verdict === "deny") return [t("plan.settled.revise"), t("plan.settled.revise_note")];
  return [t("plan.settled.elsewhere"), t("plan.settled.elsewhere_note")];
}

const props = defineProps<{
  item: Extract<Item, { t: "approval" }>;
  onPlan: (itemId: string, id: string, action: PlanAction) => Promise<void>;
}>();

const live = usePlanLive();
const sealed = computed(() => props.item.verdict !== undefined);
const started = computed(() => props.item.verdict === "start");
const latest = computed(() => !!live && live.latest === props.item.id);
const following = computed(() => latest.value && started.value && !!live && live.plan.length > 0);
const proposed = computed(() => proposedSteps(props.item.a));
const steps = computed(() => (!started.value ? proposed.value : following.value ? live!.plan : props.item.steps ?? proposed.value));
const finished = computed(() => steps.value.length > 0 && steps.value.every(stepDone));
const phase = computed<Phase>(() => {
  if (!sealed.value) return "pending";
  if (!started.value) return "settled";
  if (finished.value) return "done";
  if (latest.value && live?.blocked) return "waiting";
  if (latest.value && live?.running) return "running";
  return "stopped";
});

const submitting = ref<PlanAction | "">("");
const decide = async (action: PlanAction) => {
  if (submitting.value) return;
  submitting.value = action;
  try {
    await props.onPlan(props.item.id, props.item.a.id, action);
  } finally {
    submitting.value = "";
  }
};

const card = ref<HTMLElement | null>(null);
const current = computed(() => latest.value && started.value);
watchEffect((onCleanup) => {
  const node = card.value;
  const seen = live?.onSeen;
  if (!current.value || !node || !seen || typeof IntersectionObserver === "undefined") return;
  const watch = new IntersectionObserver(([entry]) => seen(!!entry?.isIntersecting), { root: node.closest(".scroll") });
  watch.observe(node);
  onCleanup(() => {
    watch.disconnect();
    seen(false);
  });
});

const n = computed(() => stepOrdinal(steps.value));
const at = computed(() => steps.value[currentStep(steps.value)] ?? steps.value[n.value - 1]);
const settledText = computed(() => settledWords(props.item.verdict));
const badge = computed(() => badgeOf(phase.value, n.value, steps.value.length, settledText.value[0]));
const sub = computed(() => {
  const ph = phase.value;
  const step = at.value;
  if (ph === "running" && step) return t("plan.now", { step: stepLabel(step) });
  if ((ph === "waiting" || ph === "stopped") && step) return t("plan.stopped_at", { n: n.value, step: stepLabel(step) });
  if (ph === "done") return t("plan.all_done", { n: steps.value.length });
  if (ph === "settled") return settledText.value[1];
  return null;
});
</script>

<template>
  <div class="call" data-k="plan" :data-prompt="sealed ? 'settled' : 'pending'">
    <section ref="card" class="plan-card" :data-phase="phase" :aria-label="t('plan.title')" :aria-busy="!!submitting">
      <header class="plan-card-head">
        <div class="plan-card-title">
          <b>{{ t("plan.title") }}</b>
          <span class="plan-card-badge">{{ badge }}</span>
        </div>
        <div v-if="sub" class="plan-card-sub">
          <template v-if="phase === 'running' && at"><RMark class="plan-spin" /><span>{{ sub }}</span></template>
          <template v-else>{{ sub }}</template>
        </div>
      </header>
      <PlanSteps :steps="steps" :state="STEPS[phase]" />
      <footer v-if="!sealed" class="plan-card-foot">
        <button type="button" class="plan-card-btn" data-primary="" data-action="decision.plan" :data-target="item.a.id" data-value="start"
          :disabled="!!submitting" @click="decide('start')">
          {{ submitting === "start" ? t("common.submitting") : t("plan.action.start") }}
        </button>
        <button type="button" class="plan-card-btn" data-action="decision.plan" :data-target="item.a.id" data-value="revise"
          :disabled="!!submitting" @click="decide('revise')">
          {{ t("plan.action.revise") }}
        </button>
        <button type="button" class="plan-card-btn" data-quiet="" data-action="decision.plan" :data-target="item.a.id" data-value="exit"
          :disabled="!!submitting" @click="decide('exit')">
          {{ t("plan.action.exit") }}
        </button>
        <!-- Revising is not discarding: it keeps planning, which is how a plan gets changed. -->
        <span class="plan-card-hint">{{ t("plan.revise_hint") }}</span>
      </footer>
    </section>
  </div>
</template>
