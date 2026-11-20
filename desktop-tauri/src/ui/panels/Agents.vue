<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the Agents panel needs from the rail. */
export interface AgentsPanelProps {
  port: AgentPort;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import { elapsed } from "../../i18n/format";
import type { SubagentRun, SubagentState } from "../../port/subagent";
import type { ProcessEntry } from "../../port/process";
import { delegateStateLabel } from "../delegation";
import { Grp, Row } from "./kit";

// The rail reports, it does not enumerate: a finished run is history, so the
// tail is what a long session keeps.
const SHOWN = 40;
const POLL_MS = 2000;

const props = defineProps<AgentsPanelProps>();

// The kernel is the authority on what this conversation spawned, so this reads
// its list rather than re-deriving delegates from tool cards: one fleet call
// opens many runs, and a card is a single drawing of it.
const runs = ref<SubagentRun[]>([]);
const held = ref<ProcessEntry[]>([]);
const failed = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;

const read = () => {
  clearTimeout(timer);
  Promise.all([props.port.subagents(), props.port.processes()])
    .then(([next, procs]) => {
      runs.value = next;
      held.value = procs;
      failed.value = "";
      // Poll only while something moves: a settled list asked again forever is
      // a request per two seconds against a session that has nothing to say.
      if (next.some((r) => r.state === "running")) timer = setTimeout(read, POLL_MS);
    })
    .catch((e: unknown) => {
      // A pane that is not the session's own is refused this list, and saying so
      // as an empty panel would claim "nothing is running" — a different answer.
      // The poll stops because the refusal will not change by itself.
      failed.value = e instanceof Error ? e.message : String(e);
      props.onError(e);
    });
};

// Re-read when the port changes and stop when the panel goes: a loop that
// outlives its panel keeps asking against a session nobody is showing.
watch(() => props.port, read, { immediate: true });
onUnmounted(() => clearTimeout(timer));

const live = computed(() => runs.value.filter((r) => r.state === "running").length);
const failures = computed(() => runs.value.filter((r) => r.state === "failed").length);
// A run waiting for a slot is neither running nor failed: it is the fan-out the
// concurrency ceiling is holding, which no row here could say before.
const queued = computed(() => held.value.filter((p) => p.state === "queued" || p.state === "pending").length);
const shown = computed(() => (runs.value.length > SHOWN ? runs.value.slice(-SHOWN) : runs.value));

// A run's own state is the verdict; the outline tone follows it the way the
// delegate rows in the transcript do, so the two readings agree.
const settled = (state: SubagentState) => (state === "completed" ? "" : undefined);
const broken = (state: SubagentState) => (state === "failed" || state === "cancelled" ? "" : undefined);
const took = (r: SubagentRun) =>
  elapsed(Math.max(0, (r.state === "running" ? Date.now() : r.updatedAt) - r.startedAt));
</script>

<template>
  <!-- A conversation that delegated nothing has nothing to say about delegates;
       a block whose whole content is that it has none is a standing zero. -->
  <Grp v-if="runs.length > 0 || failed" id="agents" :name="t('common.subagents')">
    <Row :k="t('subagent.count')" :v="t('subagent.live_running_total_total', { live, total: runs.length })" />
    <Row v-if="queued > 0" :k="t('subagent.process.state.queued')" :v="queued" tone="warn" />
    <Row v-if="failures > 0" :k="t('common.failed')" :v="failures" tone="err" />
    <p v-if="failed" class="ctxnote" role="alert">{{ failed }}</p>
    <div v-else class="agents">
      <div v-for="r in shown" :key="r.ref" class="ag" :data-status="r.state">
        <i class="pip" :data-settled="settled(r.state)" :data-failed="broken(r.state)"
          :style="r.state === 'running' ? { background: 'var(--net)', animation: 'tick 1.6s ease-in-out infinite' } : undefined" />
        <span class="nm">{{ r.name || r.kind }}</span>
        <span v-if="r.model" class="lbl" :title="[r.model, r.effort].filter(Boolean).join(' · ')">{{ r.model.split('/').pop() }}</span>
        <span class="rt">{{ delegateStateLabel(r.state) }} · {{ took(r) }}</span>
      </div>
    </div>
  </Grp>
</template>
