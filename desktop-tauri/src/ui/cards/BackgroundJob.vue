<script lang="ts">
import { defineComponent, h, inject, onScopeDispose, ref, watch } from "vue";
import { t } from "../../i18n";
import { JobsLiveContext } from "../JobRunView.vue";
import { jobElapsed } from "../panels/jobElapsed";

/** Where the job a call sent to the background stands, read from the pane's
 *  live job list rather than from what the call said when it started. */
export const BackgroundReading = defineComponent({
  name: "BackgroundReading",
  props: { id: { type: String, required: true } },
  setup(props) {
    const live = inject(JobsLiveContext, { jobs: [] });
    const tick = ref(0);
    let timer: ReturnType<typeof setInterval> | undefined;
    const stop = () => { if (timer !== undefined) { clearInterval(timer); timer = undefined; } };
    watch(() => live.jobs.find((j) => j.id === props.id)?.status, (status) => {
      stop();
      if (status !== "running") return;
      timer = setInterval(() => (tick.value += 1), 1000);
    }, { immediate: true });
    onScopeDispose(stop);
    return () => {
      void tick.value;
      const job = live.jobs.find((j) => j.id === props.id);
      if (!job) return null;
      const running = job.status === "running";
      const text = running
        ? t("chat.background_running", { time: jobElapsed(Date.now() - job.startedAt) })
        : [job.endedAt ? jobElapsed(job.endedAt - job.startedAt) : "", job.exitCode != null ? t("panels.exit_code", { code: job.exitCode }) : ""].filter(Boolean).join(" · ");
      return text ? h("span", { class: "tool-bg", "data-status": job.status }, text) : null;
    };
  },
});

/** While the job runs, what sending it away means, and the way to its output. */
export const BackgroundNote = defineComponent({
  name: "BackgroundNote",
  props: { id: { type: String, required: true } },
  setup(props) {
    const live = inject(JobsLiveContext, { jobs: [] });
    return () => {
      const job = live.jobs.find((j) => j.id === props.id);
      if (job?.status !== "running") return null;
      return h("p", { class: "tool-bg-note" }, [
        t("chat.background_note"),
        live.open ? h("button", { type: "button", "data-action": "job.view", "data-target": props.id, onClick: () => live.open?.(job) }, t("panels.view_output")) : null,
      ]);
    };
  },
});
export default BackgroundReading;
</script>
