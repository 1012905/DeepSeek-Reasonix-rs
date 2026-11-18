<script lang="ts">
import type { AgentPort } from "../port/port";
import type { SubagentRun } from "../port/subagent";

export interface AgentRunViewProps {
  port: AgentPort;
  run: Pick<SubagentRun, "ref" | "name">;
  shown: boolean;
  onShowCall?: (callId: string) => void;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { fromHistory } from "../state/session";
import type { HistoryMessage } from "../port/port";
import { delegateStateLabel } from "./delegation";
import Transcript from "./Transcript.vue";
import { AgentAvatar } from "./cards/SubagentGroup.vue";

/** A sub-agent run read from the kernel's own record, so what it shows does not
 *  depend on what the progress stream happened to carry. A running run is polled
 *  from where the last page ended; a finished one is read once. */
const props = defineProps<AgentRunViewProps>();

const POLL_MS = 1500;
const noop = async () => undefined as never;
const none = {
  entering: [] as string[], onEntered: () => {}, waiting: {}, onExtInvoke: () => {},
  checkpoints: new Map(), onOpenProject: () => {}, onKeepHere: () => {}, takeovers: {}, needsProject: false,
  onApprove: noop, onFullAccess: noop, onPlan: noop, onAnswer: noop, onForget: noop, onExtSubmit: noop,
  onPrepareRewind: noop, onCommitRewind: noop, onPrepareFileRevert: noop, onCommitFileRevert: noop,
};

const messages = ref<HistoryMessage[]>([]);
const info = ref<SubagentRun | undefined>(undefined);
const failed = ref("");
// Transcript takes the Ref itself; a top-level setup ref reaches the template unwrapped.
const flow = { scroll: ref<HTMLDivElement | null>(null) };

watch([() => props.port, () => props.run.ref, () => props.shown], ([port, runRef, shown], _old, onCleanup) => {
  let current = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  messages.value = [];
  info.value = undefined;
  failed.value = "";
  const read = async (after: number) => {
    try {
      const page = await port.subagentTranscript(runRef, after);
      if (!current) return;
      info.value = page.run;
      failed.value = "";
      if (page.messages.length) messages.value = after === 0 ? page.messages : [...messages.value, ...page.messages];
      if (page.next < page.total) void read(page.next);
      else if (page.live) timer = setTimeout(() => void read(page.next), POLL_MS);
    } catch (e) {
      if (current) failed.value = reason(e);
    }
  };
  if (shown) void read(0);
  onCleanup(() => {
    current = false;
    clearTimeout(timer);
  });
}, { immediate: true });

// The run's first user message is the task the main session handed over; it
// heads the tab instead of reading as a line of the run.
const all = computed(() => fromHistory(messages.value).items);
const task = computed(() => {
  const first = all.value[0];
  return first?.t === "user" ? first.text : "";
});
const items = computed(() => (task.value ? all.value.slice(1) : all.value));
const status = computed(() => (info.value ? delegateStateLabel(info.value.state) : ""));
const call = computed(() => info.value?.parentCall);
const canStop = computed(() => info.value?.state === "running" && !!call.value && !!props.port.stopToolCall);
const stop = () => { if (call.value) void props.port.stopToolCall(call.value).catch(() => {}); };
const back = () => { if (call.value) props.onShowCall?.(call.value); };
</script>

<template>
  <div class="agentrun" :data-state="info?.state">
    <div class="agentrun-hd">
      <AgentAvatar :name="run.name" />
      <span class="who">{{ run.name }}</span>
      <span v-if="info?.model" class="model">{{ info.model }}</span>
      <button v-if="canStop" type="button" class="agentrun-stop" data-action="agent.stop" @click="stop">{{ t("subagent.tab.stop") }}</button>
      <span v-else-if="status" class="rt">{{ status }}</span>
    </div>
    <div v-if="task" class="agentrun-task">
      <b>{{ t("subagent.tab.task") }}</b>
      <p>{{ task }}</p>
    </div>
    <div v-if="failed" class="workbench-error" role="alert">{{ failed }}</div>
    <div v-if="!failed && !info" class="workbench-empty">{{ t("common.loading") }}</div>
    <Transcript
      v-if="info"
      v-bind="none"
      :items="items"
      :port="port"
      :scroll="flow.scroll"
      :revision="messages.length"
      :hidden="false"
      :focus="null"
      :turn-running="info.state === 'running'"
    />
    <p v-if="info" class="agentrun-note">
      {{ t("subagent.tab.read_only") }}
      <button v-if="call && onShowCall" type="button" data-action="agent.show-call" @click="back">{{ t("subagent.tab.back") }}</button>
    </p>
  </div>
</template>