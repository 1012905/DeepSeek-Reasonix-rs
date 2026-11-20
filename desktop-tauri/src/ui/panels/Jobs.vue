<script lang="ts">
import type { AgentPort, JobEntry } from "../../port/port";

/** What the Jobs panel needs from the rail. */
export interface JobsPanelProps {
  port: AgentPort;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import JobName, { jobName } from "./JobName.vue";
import { jobElapsed } from "./jobElapsed";
import JobRunView, { JobsLive } from "../JobRunView.vue";
import StudioIcon from "../StudioIcon.vue";

// How an ended job finished, by the kernel's own status: only "done" is a
// success, and a job someone stopped is neither a success nor a fault.
const ENDED: Record<string, { label: string; tone: string }> = {
  done: { label: "common.done_2", tone: "var(--ok)" },
  failed: { label: "common.failed", tone: "var(--err)" },
  killed: { label: "plan.stopped", tone: "var(--faint)" },
  interrupted: { label: "common.interrupted", tone: "var(--faint)" },
};

// A confirmation nobody has to dismiss: it puts itself away if the reader
// thought better of it, which is what keeps it from being a dialog in disguise.
const ARMED_MS = 3000;
const POLL_MS = 1000;

const props = defineProps<JobsPanelProps>();

// The status call is what carries the job list — the kernel keeps running and
// ended jobs apart there, and a panel building its own list would lose the
// distinction the moment a job crossed it.
const runningJobs = ref<JobEntry[]>([]);
const endedJobs = ref<JobEntry[]>([]);
const note = ref("");
const stopping = ref<ReadonlySet<string>>(new Set());
const armed = ref("");
const viewing = ref("");
// Elapsed readings are recomputed against a clock this panel owns; a job list
// re-rendered only when the kernel answers would show a frozen second hand.
const clock = ref(0);

let poll: ReturnType<typeof setTimeout> | undefined;
let armTimer: ReturnType<typeof setTimeout> | undefined;
let ticker: ReturnType<typeof setInterval> | undefined;

const read = () => {
  clearTimeout(poll);
  props.port
    .status()
    .then((st) => {
      runningJobs.value = st.jobs ?? [];
      endedJobs.value = st.endedJobs ?? [];
      note.value = "";
      // Poll only while something runs: a settled list asked again forever is a
      // request a second against a session with nothing left to say.
      if (runningJobs.value.length > 0) poll = setTimeout(read, POLL_MS);
    })
    .catch((e: unknown) => {
      note.value = reason(e);
      props.onError(e);
    });
};

watch(() => props.port, read, { immediate: true });

// One timer for the whole panel, started only while some job is running: an
// idle rail must not tick.
watch(runningJobs, (jobs) => {
  clearInterval(ticker);
  if (jobs.length === 0) return;
  ticker = setInterval(() => (clock.value += 1), 1000);
}, { immediate: true });

onUnmounted(() => {
  clearTimeout(poll);
  clearInterval(ticker);
  if (armTimer) clearTimeout(armTimer);
});

const arm = (id: string) => {
  armed.value = id;
  if (armTimer) clearTimeout(armTimer);
  armTimer = setTimeout(() => { armed.value = ""; }, ARMED_MS);
};

const cancel = (id: string) => {
  armed.value = "";
  stopping.value = new Set(stopping.value).add(id);
  props.port
    .cancelJob(id)
    // A job that already ended is refused with a code rather than silently
    // acknowledged, so the answer is re-read rather than assumed.
    .catch((e: unknown) => { note.value = reason(e); props.onError(e); })
    .finally(() => {
      const next = new Set(stopping.value);
      next.delete(id);
      stopping.value = next;
      read();
    });
};

const all = computed(() => [...runningJobs.value, ...endedJobs.value]);
const nameOf = (id: string) => {
  const job = all.value.find((j) => j.id === id);
  return job ? jobName(job) : id;
};
// One reading per row: how long it has run, or how it ended. A running row that
// printed both said the same second twice, and an ended row with no endedAt
// would otherwise draw a duration nobody measured.
const stateOf = (j: JobEntry) => {
  void clock.value;
  if (j.status === "running") return jobElapsed(Date.now() - j.startedAt);
  return j.endedAt ? jobElapsed(j.endedAt - j.startedAt) : t(ENDED[j.status]?.label ?? "common.done_2");
};
</script>

<template>
  <div v-if="all.length > 0 || note" class="block" data-b="jobs">
    <div class="lbl">
      <span>{{ t("common.background_jobs") }}</span>
      <span class="c">
        {{ runningJobs.length > 0 ? t("panels.running_n", { n: runningJobs.length }) : t("panels.ended_n", { n: endedJobs.length }) }}
      </span>
    </div>
    <p v-if="note" class="ctxnote" role="alert">{{ note }}</p>

    <div class="jobs">
      <div v-for="j in all" :key="j.id" class="job" :data-done="j.status !== 'running' ? '' : undefined" :data-status="j.status">
        <i class="pip" :data-settled="j.status === 'running' ? undefined : ''"
          :data-failed="j.status === 'failed' ? '' : undefined"
          :style="j.status === 'running'
            ? { background: 'var(--net)', animation: 'tick 1.6s ease-in-out infinite' }
            : { background: ENDED[j.status]?.tone ?? 'var(--faint)' }" />
        <!-- Both a running and an ended job open their output: after the fact is
             when it is usually wanted, and this is the only door to it. -->
        <JobName :job="j">
          <button type="button" class="cmd job-view" :aria-label="t('panels.view_output_cmd', { cmd: jobName(j) })"
            :aria-expanded="viewing === j.id" @click="viewing = viewing === j.id ? '' : j.id">
            {{ jobName(j) }}
          </button>
        </JobName>
        <span class="rt">{{ stateOf(j) }}</span>
        <template v-if="j.status === 'running'">
          <span v-if="armed === j.id" class="job-confirm">
            <span>{{ t("panels.stop") }}</span>
            <button type="button" class="job-stop" :disabled="stopping.has(j.id)"
              :aria-label="t('panels.confirm_stopping_background_task_cmd', { cmd: jobName(j) })" @click="cancel(j.id)">
              {{ t("panels.ok") }}
            </button>
          </span>
          <button v-else type="button" class="job-stop" :disabled="stopping.has(j.id)" :title="t('common.stop')"
            :aria-label="t('panels.stop_background_job_cmd', { cmd: jobName(j) })" @click="arm(j.id)">
            <StudioIcon name="stop" />
          </button>
        </template>
      </div>
    </div>

    <!-- The output view is the pane's own JobRunView, under the row that asked
         for it. It reads the job through JobsLive rather than through props, so
         this is the only place the provider has to exist. -->
    <JobsLive v-if="viewing" :jobs="all" :cancel="(id: string) => port.cancelJob(id)">
      <div class="jobrun jobrun-inline">
        <JobRunView :port="port" :id="viewing" :name="nameOf(viewing)" :shown="true" />
      </div>
    </JobsLive>
  </div>
</template>
