<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the Runtime panel needs from the rail. */
export interface RuntimePanelProps {
  port: AgentPort;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import type { SessionUsage } from "../../port/session";
import type { Decision } from "../../port/session";
import { useTrail } from "../num";
import Spark from "../Spark.vue";
import { Grp, Row } from "./kit";

// The rate is one sample a second over the tail of the turn, read off the
// session's own ledger rather than off the token deltas: a delta is text, and
// esticounting text would put a figure on screen that no request produced.
const SAMPLE_MS = 1000;
const TRAIL = 60;

const props = defineProps<RuntimePanelProps>();

const usage = ref<SessionUsage | null>(null);
const running = ref(false);
const waiting = ref<number>(0);
const note = ref("");

const sample = ref(0);
const trail = useTrail(sample, running, TRAIL);

let timer: ReturnType<typeof setTimeout> | undefined;
let sampler: ReturnType<typeof setInterval> | undefined;
let lastCompletion = 0;

const read = () => {
  clearTimeout(timer);
  props.port
    .status()
    .then((st) => {
      usage.value = st.sessionUsage ?? null;
      running.value = !!st.running;
      waiting.value = pending(st.decisions);
      note.value = "";
      if (running.value) timer = setTimeout(read, SAMPLE_MS);
    })
    .catch((e: unknown) => {
      note.value = e instanceof Error ? e.message : String(e);
      props.onError(e);
    });
};

// The decisions list is what says a person is being waited on: `decisions` is
// defined as what is still unanswered, and all four kinds need one.
const pending = (list: Decision[] | undefined) => (list ?? []).length;

watch(() => props.port, read, { immediate: true });

// The sparkline's point is the delta in output tokens since the previous second,
// not the running total: a total only ever climbs, which reads as a rate that is
// always rising.
watch(running, (on) => {
  clearInterval(sampler);
  lastCompletion = usage.value?.completionTokens ?? 0;
  if (!on) {
    sample.value = 0;
    return;
  }
  sampler = setInterval(() => {
    const total = usage.value?.completionTokens ?? 0;
    sample.value = Math.max(0, total - lastCompletion);
    lastCompletion = total;
  }, SAMPLE_MS);
}, { immediate: true });

onUnmounted(() => {
  clearTimeout(timer);
  clearInterval(sampler);
});

// Output tokens per second over the sampling window. A single sample is not a
// rate — one point in time has no slope — so the trail's own span is the divisor.
const rate = computed(() => {
  const points = trail.value;
  if (points.length < 2) return 0;
  return points.reduce((n, v) => n + v, 0) / points.length;
});

const work = computed(() => usage.value?.workMs ?? 0);
const speech = computed(() => running.value || waiting.value > 0 || !!(usage.value?.requests));
</script>

<template>
  <!-- A registry of zeroes spends the same width on a clean run as on a broken
       one, and a reader who has learned to skip six of them skips the seventh. -->
  <Grp v-if="speech || note" id="runtime" :name="t('panels.run_metrics')"
    :aside="usage ? t('panels.n_calls', { n: usage.requests }) : undefined">
    <!-- The one fact here a reader can act on, and the only one about them
         rather than about the run, so it leads and is toned as a request. -->
    <Row v-if="waiting > 0" :k="t('panels.awaiting_you')" :v="waiting" tone="act" />
    <!-- Throughput is about a turn in flight; once one is over it has nothing
         left to say. -->
    <div v-if="running" class="drow">
      <span class="k">{{ t("panels.throughput_output") }}</span>
      <span class="v">
        <span class="rate" :data-live="rate >= 1 ? '' : undefined">
          <Spark :points="trail" />
          <span class="v">{{ rate >= 1 ? Math.round(rate) : "—" }}</span>
          <span class="u">tok/s</span>
        </span>
      </span>
    </div>
    <Row v-if="usage && usage.requests > 0" :k="t('panels.model_round')" :v="usage.requests" />
    <Row v-if="work > 0" :k="t('panels.total_time')" :v="`${Math.round(work / 1000)}s`" />
    <p v-if="note" class="ctxnote" role="alert">{{ note }}</p>
  </Grp>
</template>
