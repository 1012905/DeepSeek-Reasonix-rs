<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type {
  AgentPort, ScheduleBoard, ScheduleDraft, SchedulePreview, ScheduleResultView, ScheduleRunView, ScheduleView,
} from "../../../port/port";
import Group from "../../Group.vue";

// Read-only tasks a person confirmed, run at a time they chose. A creation is
// shown back as a preview with the most it can spend, and only that preview's
// own confirmation creates it. A report is the model's text: it is drawn as
// plain text, never as markup.
const EVERY: readonly (readonly [number, string])[] = [
  [3600, "panels.every_hour"],
  [7200, "panels.every_2_hours"],
  [21600, "panels.every_6_hours"],
  [43200, "panels.every_12_hours"],
  [86400, "panels.every_day"],
  [604800, "panels.every_week"],
];
const POLL_MS = 30_000;

const props = defineProps<{ port: AgentPort; onChanged?: () => void }>();

const when = (unix: number) => new Date(unix * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

const STATUS: Record<string, string> = { active: "common.running", paused: "app.held", expired: "panels.expired", "circuit-open": "panels.stopped_by_the_safety_breaker" };
const PAUSE: Record<string, string> = {
  user: "panels.you_paused_it",
  consecutive_failures: "panels.paused_automatically_after_repeated_failures",
  breaker: "panels.paused_automatically_to_protect_the",
  lifetime_budget: "panels.the_lifetime_budget_is_used",
  corrupt: "panels.its_record_is_damaged",
};
const RUN: Record<string, string> = {
  succeeded: "common.done_2", failed: "common.failed", blocked: "panels.waiting_for_you", budget_stopped: "panels.budget_used_up",
  interrupted: "panels.interrupted", skipped: "panels.skipped_2", claimed: "panels.about_to_start", running: "common.running",
};
const SKIP: Record<string, string> = { missed: "panels.missed_its_time", budget_window: "panels.the_budget_window_is_full", lease_held: "panels.another_run_was_in_progress", concurrency: "panels.another_run_was_in_progress" };

const board = ref<ScheduleBoard | null>(null);
const busy = ref(false);
const error = ref("");
const loadError = ref("");
const readOnly = ref(false);
const form = ref<ScheduleDraft | null>(null);
const preview = ref<SchedulePreview | null>(null);
const confirmDelete = ref("");
const result = ref<ScheduleResultView | null>(null);

let alive = true;
let timer: ReturnType<typeof setInterval> | null = null;

// A failed read keeps the last board and says why: the block vanishing would
// leave a person who just created a schedule unable to tell whether it exists.
async function refresh() {
  try {
    const next = await props.port.schedules();
    if (!alive) return;
    board.value = next;
    loadError.value = "";
  } catch (e) {
    if (alive) loadError.value = reason(e);
  }
}

watch(() => props.port, (_next, _was, onCleanup) => {
  alive = true;
  void refresh();
  timer = setInterval(() => { if (typeof document === "undefined" || !document.hidden) void refresh(); }, POLL_MS);
  onCleanup(() => { alive = false; if (timer) clearInterval(timer); timer = null; });
}, { immediate: true });

async function run(work: () => Promise<unknown>) {
  busy.value = true;
  error.value = "";
  try {
    await work();
    await refresh();
    props.onChanged?.();
  } catch (e) {
    if (e instanceof HttpError && e.reason?.code === "schedule.editing_disabled") readOnly.value = true;
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

const locked = computed(() => busy.value || readOnly.value);
const name = (s: ScheduleView) => s.label || base(s.workspace);
const scheduleName = (r: ScheduleRunView) => {
  const found = board.value?.schedules.find((x) => x.id === r.scheduleId);
  return found ? name(found) : r.scheduleId;
};

function trigger(s: ScheduleView): string {
  if (s.atUnix) return t("panels.once_at", { at: when(s.atUnix) });
  const hit = EVERY.find(([sec]) => sec === s.everySec);
  return hit ? t(hit[1]) : t("panels.every_n_minutes", { n: Math.round((s.everySec ?? 0) / 60) });
}

const hint = computed(() => {
  const head = t("settings.scheduled_runs_hint");
  const items = board.value?.schedules;
  if (!items) return head;
  const paused = items.filter((s) => s.status === "paused").length;
  return head + ` · ${t("settings.n_scheduled", { n: items.length })}${paused ? t("settings.n_paused", { n: paused }) : ""}`;
});

const patch = (p: Partial<ScheduleDraft>) => { if (form.value) form.value = { ...form.value, ...p }; };
const onPick = (key: "workspace" | "everySec") => (e: Event) => {
  const value = (e.target as HTMLSelectElement).value;
  patch(key === "everySec" ? { everySec: Number(value) } : { workspace: value });
};
const onText = (key: "prompt" | "label") => (e: Event) => patch({ [key]: (e.target as HTMLTextAreaElement | HTMLInputElement).value });

const startNew = () => { error.value = ""; preview.value = null; form.value = { label: "", everySec: 86400, workspace: "", prompt: "" }; };
const askPreview = () => void run(async () => {
  if (!form.value) return;
  preview.value = await props.port.previewSchedule({ ...form.value, workspace: form.value.workspace.trim(), prompt: form.value.prompt.trim() });
});
const create = () => void run(async () => {
  if (!form.value || !preview.value) return;
  await props.port.createSchedule({ ...form.value, workspace: form.value.workspace.trim(), prompt: form.value.prompt.trim() }, preview.value.confirm);
  form.value = null;
  preview.value = null;
});
const showResult = (r: ScheduleRunView) => void run(async () => { result.value = await props.port.scheduleResult(r.triggerId); });
</script>

<template>
  <Group id="scheduled-runs" :title="t('panels.scheduled_runs')" :hint="hint">
    <div class="box">
      <div v-if="loadError" class="why" role="alert">
        {{ t(board ? "panels.could_not_refresh_the_scheduled" : "panels.could_not_read_the_scheduled", { reason: loadError }) }}
        <span class="acts"><button class="act" data-action="schedule.retry" @click="refresh">{{ t("common.retry") }}</button></span>
      </div>

      <template v-if="board">
        <div class="lrow">
          <span class="tx">
            <span class="lb">{{ t(board.pausedAll ? "panels.all_scheduled_runs_are_paused" : "panels.scheduled_runs_are_on") }}</span>
            <span class="ds">{{ t("panels.at_most_n_at_least", { n: board.policy.maxSchedules, m: board.policy.minIntervalMinutes, k: board.policy.globalTokensDay.toLocaleString() }) }}</span>
          </span>
          <button v-if="board.pausedAll" class="act" data-action="schedule.resume-all" :disabled="locked" @click="run(() => port.setSchedulesPaused(false))">{{ t("panels.resume_all") }}</button>
          <button v-else class="act" data-action="schedule.pause-all" :disabled="busy" @click="run(() => port.setSchedulesPaused(true))">{{ t("panels.pause_all") }}</button>
        </div>

        <div v-if="board.schedules.length === 0" class="peernote">{{ t("panels.no_scheduled_runs_yet") }}</div>
        <ul v-else class="wa-list" :aria-label="t('panels.scheduled_runs_2')">
          <li v-for="s in board.schedules" :key="s.id">
            <span class="tx">
              <span class="lb">{{ name(s) }}</span>
              <span class="ds">{{ trigger(s) }} · {{ t(STATUS[s.status] ?? s.status) }}{{ s.pausedReason ? " · " + t(PAUSE[s.pausedReason] ?? s.pausedReason) : "" }}</span>
              <span class="ds">{{ s.workspace }} · {{ s.model }}</span>
              <span class="ds">{{ t("panels.used_a_of_b_tokens", { a: s.spentLifetimeTokens.toLocaleString(), b: s.lifetimeTokens.toLocaleString(), d: when(s.expiresAtUnix) }) }}</span>
            </span>
            <span class="acts">
              <button v-if="s.status === 'active'" class="act" data-action="schedule.pause" :data-target="s.id" :disabled="busy" @click="run(() => port.scheduleAction(s.id, 'pause'))">{{ t("panels.pause") }}</button>
              <button v-else class="act" data-action="schedule.resume" :data-target="s.id" :disabled="locked" @click="run(() => port.scheduleAction(s.id, 'resume'))">{{ t("panels.resume") }}</button>
              <button class="act" data-action="schedule.renew" :data-target="s.id" :disabled="locked" @click="run(() => port.scheduleAction(s.id, 'renew'))">{{ t("panels.renew") }}</button>
              <button v-if="confirmDelete === s.id" class="act danger" data-action="schedule.delete" :data-target="s.id" :disabled="busy"
                @click="run(async () => { await port.scheduleAction(s.id, 'delete'); confirmDelete = ''; })">{{ t("common.confirm_delete") }}</button>
              <button v-else class="act ghost" data-action="schedule.delete-ask" :data-target="s.id" :disabled="busy" @click="confirmDelete = s.id">{{ t("common.delete") }}</button>
            </span>
          </li>
        </ul>

        <template v-if="board.runs.length > 0">
          <div class="peernote"><b>{{ t("panels.recent_runs") }}</b></div>
          <ul class="wa-list" :aria-label="t('panels.recent_runs')">
            <li v-for="r in board.runs.slice(0, 8)" :key="r.triggerId">
              <span class="tx">
                <span class="lb">{{ scheduleName(r) }} · {{ t(RUN[r.state] ?? r.state) }}</span>
                <span class="ds">{{ when(r.slotUnix) }} · {{ r.chargedTokens.toLocaleString() }} {{ t("panels.tokens") }}{{ r.skipReason ? " · " + t(SKIP[r.skipReason] ?? r.skipReason) : "" }}</span>
              </span>
              <button v-if="r.state !== 'skipped' && r.state !== 'claimed'" class="act" data-action="schedule.result" :data-target="r.triggerId" :disabled="busy" @click="showResult(r)">{{ t("panels.view_result") }}</button>
            </li>
          </ul>
        </template>

        <div v-if="result" class="peerform" role="region" :aria-label="t('panels.result')">
          <div class="peernote"><b>{{ t("panels.result") }}</b> · {{ when(result.recordedUnix) }}</div>
          <div class="peernote">{{ t("panels.the_text_below_was_written") }}</div>
          <pre class="peerbody" data-testid="schedule-report">{{ result.report }}</pre>
          <div v-if="result.reportTruncated" class="peernote">{{ t("panels.the_report_was_too_long") }}</div>
          <template v-if="result.pending.length > 0">
            <div class="peernote"><b>{{ t("panels.waiting_for_your_decision") }}</b></div>
            <div class="peernote">{{ t("panels.the_text_below_was_written") }}</div>
            <ul class="wa-list" data-testid="schedule-pending">
              <li v-for="(p, i) in result.pending" :key="i">
                <span class="tx"><span class="lb">{{ p.summary }}</span><span v-if="p.detail" class="ds">{{ p.detail }}</span></span>
              </li>
            </ul>
            <div class="peernote">{{ t("panels.none_of_these_was_run") }}</div>
          </template>
          <span class="acts"><button class="act ghost" data-action="schedule.result-close" @click="result = null">{{ t("common.close") }}</button></span>
        </div>

        <div v-if="!form" class="peerform">
          <span class="acts">
            <button class="act" data-action="schedule.new" :disabled="locked || board.schedules.length >= board.policy.maxSchedules" @click="startNew">{{ t("panels.new_scheduled_run") }}</button>
          </span>
        </div>

        <div v-else-if="!preview" class="peerform" role="group" :aria-label="t('panels.new_scheduled_run')">
          <label class="peernote" for="sched-folder">{{ t("panels.folder_it_can_only_read") }}</label>
          <div v-if="board.current?.trustable" class="peernote">
            {{ t(board.current.trust === "declined" ? "panels.the_open_project_name_was" : "panels.the_open_project_name_is", { name: board.current.name }) }}
            <button class="act" data-action="workspace.trust" data-value="trusted" :disabled="busy" @click="run(() => port.decideWorkspaceTrust('trusted'))">{{ t("common.trust_this_folder") }}</button>
          </div>
          <div v-if="board.folders.length === 0" class="peernote" id="sched-folder">{{ t("panels.there_is_no_trusted_project") }}</div>
          <select v-else id="sched-folder" class="peerfield" data-action-change="schedule.folder" :value="form.workspace" :disabled="busy" @change="onPick('workspace')">
            <option value="">{{ t("panels.choose_a_trusted_project") }}</option>
            <option v-for="f in board.folders" :key="f.path" :value="f.path">{{ `${f.name} — ${f.path}` }}</option>
          </select>
          <label class="peernote" for="sched-prompt">{{ t("panels.what_to_do") }}</label>
          <textarea id="sched-prompt" class="peerfield" data-action-change="schedule.prompt" :value="form.prompt" :disabled="busy" @input="onText('prompt')" />
          <label class="peernote" for="sched-label">{{ t("panels.name_optional") }}</label>
          <input id="sched-label" class="peerfield" data-action-change="schedule.label" :value="form.label" :disabled="busy" @input="onText('label')" />
          <label class="peernote" for="sched-every">{{ t("panels.how_often") }}</label>
          <select id="sched-every" class="peerfield" data-action-change="schedule.every" :value="form.everySec ?? 86400" :disabled="busy" @change="onPick('everySec')">
            <option v-for="[sec, label] in EVERY" :key="sec" :value="sec" :disabled="sec < board.policy.minIntervalMinutes * 60">{{ t(label) }}</option>
          </select>
          <span class="acts">
            <button class="act ghost" data-action="schedule.cancel" :disabled="busy" @click="form = null">{{ t("common.cancel") }}</button>
            <button class="act" data-action="schedule.preview" data-primary="true" :disabled="busy || !form.workspace.trim() || !form.prompt.trim()" @click="askPreview">{{ t("common.preview") }}</button>
          </span>
        </div>

        <div v-else class="peerform" role="group" :aria-label="t('panels.confirm_and_create')">
          <div class="peernote"><b>{{ t("panels.please_confirm") }}</b></div>
          <ul class="wa-list">
            <li><span class="tx"><span class="lb">{{ t("panels.first_run") }}</span><span class="ds">{{ when(preview.firstRunUnix) }}</span></span></li>
            <li><span class="tx"><span class="lb">{{ t("panels.folder") }}</span><span class="ds">{{ preview.workspace }} {{ t("panels.read_only_it_can_read") }}</span></span></li>
            <li><span class="tx"><span class="lb">{{ t("common.models") }}</span><span class="ds">{{ preview.model }}</span></span></li>
            <li><span class="tx"><span class="lb">{{ t("panels.what_to_do") }}</span><span class="ds">{{ preview.prompt }}</span></span></li>
            <li><span class="tx"><span class="lb">{{ t("panels.each_run_at_most") }}</span><span class="ds">{{ t("panels.a_tokens_b_steps_c", { a: preview.perRunTokens.toLocaleString(), b: preview.perRunSteps, c: preview.perRunWallSec }) }}</span></span></li>
            <li><span class="tx"><span class="lb">{{ t("panels.in_total_at_most") }}</span><span class="ds">{{ t("panels.a_tokens_a_day_b", { a: preview.perDayMax.toLocaleString(), b: preview.lifetimeTokens, d: when(preview.expiresAtUnix) }) }}</span></span></li>
          </ul>
          <div class="peernote">{{ t("panels.anything_that_needs_your_approval") }}</div>
          <span class="acts">
            <button class="act ghost" data-action="schedule.back" :disabled="busy" @click="preview = null">{{ t("panels.back") }}</button>
            <button class="act" data-action="schedule.create" data-primary="true" :disabled="locked" @click="create">{{ t("panels.confirm_and_create") }}</button>
          </span>
        </div>

        <div v-if="readOnly" class="peernote" role="status">{{ t("common.these_settings_are_read_only") }}</div>
      </template>

      <div v-if="error" class="why" role="alert">{{ error }}</div>
    </div>
  </Group>
</template>
