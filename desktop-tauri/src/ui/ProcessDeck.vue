<script setup lang="ts">
import { t } from "../i18n";
import type { ProcessEntry } from "../port/process";
import { jobElapsed } from "./panels/jobElapsed";

/** What this session has running now, one row per process. State and cause are
 *  the kernel's words for it; a value this page has no wording for is shown as
 *  it came rather than guessed at. */
defineProps<{ rows: ProcessEntry[]; error: string }>();

function stateLabel(state: string): string {
  switch (state) {
    case "pending": return t("subagent.process.state.pending");
    case "queued": return t("subagent.process.state.queued");
    case "running": return t("subagent.process.state.running");
    case "waiting": return t("subagent.process.state.waiting");
    case "completed": return t("subagent.process.state.completed");
    case "failed": return t("subagent.process.state.failed");
    case "stopped": return t("subagent.process.state.stopped");
    case "interrupted": return t("subagent.process.state.interrupted");
    case "skipped": return t("subagent.process.state.skipped");
    default: return state;
  }
}

function causeLabel(cause: string): string {
  switch (cause) {
    case "slots": return t("subagent.process.cause.slots");
    case "writers": return t("subagent.process.cause.writers");
    case "claim": return t("subagent.process.cause.claim");
    case "held": return t("subagent.process.cause.held");
    case "adopted": return t("subagent.process.cause.adopted");
    case "budget": return t("subagent.process.cause.budget");
    case "parked": return t("subagent.process.cause.parked");
    default: return cause;
  }
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "subagent": return t("subagent.process.kind.subagent");
    case "job": return t("subagent.process.kind.job");
    default: return kind;
  }
}
</script>

<template>
  <div v-if="error" class="empty" role="alert">{{ error }}</div>
  <div v-else-if="rows.length === 0" class="empty">{{ t("subagent.process.empty") }}</div>
  <div v-else class="job-deck">
    <div v-for="p in rows" :key="p.id" class="job-deck-row" :data-status="p.state">
      <div class="job-deck-line">
        <b class="job-deck-name">{{ p.name }}</b>
        <span class="job-deck-time">{{ [kindLabel(p.kind), stateLabel(p.state), jobElapsed(Date.now() - p.startedAt)].join(" · ") }}</span>
      </div>
      <span v-if="p.cause" class="job-deck-tail">{{ causeLabel(p.cause) }}</span>
    </div>
  </div>
</template>