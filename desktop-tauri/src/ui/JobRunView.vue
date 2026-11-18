<script lang="ts">
import { defineComponent, inject, provide, reactive, watchEffect, type InjectionKey, type PropType } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { HttpError } from "../port/http_error";
import type { AgentPort, JobEntry, JobOutput } from "../port/port";
import { jobElapsed } from "./panels/jobElapsed";
import StudioIcon from "./StudioIcon.vue";

/** The pane's background jobs and how to stop one, for a job's workbench tab. */
export interface JobsLive { jobs: JobEntry[]; cancel?: (id: string) => Promise<void>; open?: (job: JobEntry) => void }

export const JobsLiveContext: InjectionKey<JobsLive> = Symbol("JobsLive");

/** Puts a pane's live job list in reach of every card drawn under it. */
export const JobsLive = defineComponent({
  name: "JobsLive",
  props: {
    jobs: { type: Array as PropType<JobEntry[]>, default: () => [] },
    cancel: { type: Function as PropType<((id: string) => Promise<void>) | undefined>, default: undefined },
    open: { type: Function as PropType<((job: JobEntry) => void) | undefined>, default: undefined },
  },
  setup(props, { slots }) {
    const value = reactive<JobsLive>({ jobs: [] });
    watchEffect(() => {
      value.jobs = props.jobs;
      value.cancel = props.cancel;
      value.open = props.open;
    });
    provide(JobsLiveContext, value);
    return () => slots.default?.() ?? null;
  },
});
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";

const POLL_MS = 2000;
const GONE = "job.not_running";
const FOLLOW_SLACK = 24;

/** One background job's output in the workbench. It follows the newest output
 *  until the reader scrolls up, and reading it never takes output from the
 *  model. */
const props = defineProps<{
  port: Pick<AgentPort, "jobOutput">;
  id: string;
  name: string;
  shown: boolean;
}>();

const live = inject(JobsLiveContext, { jobs: [] });
const job = computed(() => live.jobs.find((j) => j.id === props.id));
const out = ref<JobOutput | null>(null);
const failed = ref("");
const following = ref(true);
const tick = ref(0);
const box = ref<HTMLPreElement | null>(null);
const running = computed(() => (out.value?.status ?? job.value?.status) === "running");

watch([() => props.port, () => props.id, () => props.shown], ([port, id, shown], _old, onCleanup) => {
  if (!shown) return;
  let alive = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const read = () => {
    port.jobOutput(id)
      .then((next) => {
        if (!alive) return;
        out.value = next;
        failed.value = "";
        if (next.status === "running") timer = setTimeout(read, POLL_MS);
      })
      .catch((e: unknown) => {
        if (!alive || (e instanceof HttpError && e.reason?.code === GONE)) return;
        failed.value = reason(e);
        timer = setTimeout(read, POLL_MS * 2);
      });
  };
  read();
  onCleanup(() => {
    alive = false;
    clearTimeout(timer);
  });
}, { immediate: true });

watch(running, (liveNow, _old, onCleanup) => {
  if (!liveNow) return;
  const timer = setInterval(() => (tick.value += 1), 1000);
  onCleanup(() => clearInterval(timer));
}, { immediate: true });

watch([() => out.value?.text, following], () => {
  const el = box.value;
  if (el && following.value) el.scrollTop = el.scrollHeight;
}, { flush: "post" });

const onScroll = () => {
  const el = box.value;
  if (el) following.value = el.scrollHeight - el.scrollTop - el.clientHeight <= FOLLOW_SLACK;
};
const took = computed(() => {
  void tick.value;
  const j = job.value;
  if (!j) return "";
  return running.value ? jobElapsed(Date.now() - j.startedAt) : j.endedAt ? jobElapsed(j.endedAt - j.startedAt) : "";
});
const result = computed(() => {
  const j = job.value;
  if (!running.value && j) {
    return [j.exitCode != null ? t("workbench.job_exited", { code: j.exitCode }) : t("workbench.job_ended"), took.value].filter(Boolean).join(" · ");
  }
  return took.value;
});
const command = computed(() => job.value?.detail?.trim() || job.value?.label || "");
const cancel = () => { if (live.cancel) void live.cancel(props.id); };
</script>

<template>
  <div class="jobrun" :data-status="out?.status ?? job?.status">
    <div class="jobrun-hd">
      <StudioIcon name="play" class="jobrun-mark" />
      <b>{{ name }}</b>
      <span class="jobrun-state">{{ result }}</span>
      <button v-if="running && live.cancel" type="button" class="agentrun-stop" data-action="job.cancel" :data-target="id" @click="cancel">
        {{ t("common.stop") }}
      </button>
    </div>
    <p v-if="failed" class="jobrun-note" role="alert">{{ failed }}</p>
    <pre ref="box" class="jobrun-out" data-action="job.scroll" @scroll="onScroll"><span v-if="command" class="jobrun-cmd">$ {{ command }}{{ "\n" }}</span>{{ out?.text ?? "" }}</pre>
    <div v-if="running" class="jobrun-foot">
      <span>{{ following ? t("workbench.job_following") : t("workbench.job_paused") }}</span>
      <button v-if="!following" type="button" class="agent-open" data-action="job.follow" :aria-label="t('workbench.job_follow')" :title="t('workbench.job_follow')" @click="following = true">
        <StudioIcon name="down" />
      </button>
    </div>
  </div>
</template>
