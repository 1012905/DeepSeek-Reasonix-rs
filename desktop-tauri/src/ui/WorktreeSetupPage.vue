<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { HttpError } from "../port/port";
import type { WorktreePreparation } from "../port/workspace";
import StudioIcon, { type StudioIconName } from "./StudioIcon.vue";
import CopyButton from "./CopyButton.vue";
import WorkspacePath, { compactWorkspaceValue, workspaceName } from "./WorkspacePath.vue";

const phases: Record<string, string> = {
  starting: "session.submitting_creation_request", validating: "session.checking_source_branch", creating: "session.creating_workspace",
  checking_out: "session.checking_out_committed_files", copying_attachments: "session.copying_task_attachments", binding: "session.binding_workspace",
  cleaning: "session.cleaning_up_incomplete_workspace", cancelling: "session.stopping_and_cleaning_up", ready: "session.workspace_bound",
  cleanup_failed: "session.cleanup_did_not_finish", failed: "session.creation_did_not_finish", cancelled: "session.creation_stopped_and_workspace_cleaned",
  retained: "session.workspace_bound", unreachable: "session.waiting_for_confirmation",
};

const props = defineProps<{
  stage: WorktreePreparation;
  task: string;
  files: string[];
  busy: boolean;
  onRetry: () => void;
  onCancel: () => void;
  onEdit: () => void;
}>();

const now = ref(Date.now());
const details = ref(false);
const heading = ref<HTMLHeadingElement | null>(null);
watch(() => props.stage.id, () => {
  details.value = false;
  now.value = Date.now();
  heading.value?.focus();
}, { immediate: true, flush: "post" });
watch(() => props.stage.phase, () => {
  if (document.activeElement === document.body) heading.value?.focus();
}, { immediate: true, flush: "post" });
const failed = computed(() => props.stage.phase === "failed");
const cancelled = computed(() => props.stage.phase === "cancelled");
const retained = computed(() => props.stage.phase === "retained");
const uncertain = computed(() => props.stage.phase === "unreachable" || props.stage.phase === "cleanup_failed");
const ready = computed(() => props.stage.phase === "ready");
const cancelling = computed(() => props.stage.phase === "cancelling");
const active = computed(() => !failed.value && !cancelled.value && !retained.value && !ready.value && !uncertain.value);
let timer: ReturnType<typeof setInterval> | undefined;
watch(active, (isActive) => {
  clearInterval(timer);
  if (isActive) timer = setInterval(() => { now.value = Date.now(); }, 1000);
}, { immediate: true });
onUnmounted(() => clearInterval(timer));
const elapsed = computed(() => Math.max(0, Math.floor((now.value - props.stage.startedAt) / 1000)));
const slow = computed(() => active.value && !cancelling.value && elapsed.value >= 15);
const headline = computed(() => failed.value ? "session.could_not_create_worktree" : cancelled.value ? "session.creation_cancelled" : retained.value || ready.value ? "session.workspace_ready" : uncertain.value ? "session.confirm_creation_outcome" : cancelling.value ? "session.cancelling_creation" : "session.creating_worktree");
const description = computed(() => failed.value ? "session.the_task_has_not_been_2" : cancelled.value ? "session.the_task_has_not_been" : retained.value ? "session.the_workspace_was_already_created" : ready.value ? "session.opening_the_conversation_and_starting" : uncertain.value ? "session.the_operation_or_cleanup_outcome" : cancelling.value ? "session.stopping_the_operation_and_cleaning" : slow.value ? "session.creation_is_taking_longer_completion" : "session.your_task_will_start_automatically");
const error = computed(() => props.stage.code ? reason(new HttpError(409, props.stage.detail || props.stage.code, { code: props.stage.code })) : "");
const icon = computed<StudioIconName>(() => ready.value || retained.value ? "check" : failed.value || uncertain.value ? "warning" : cancelled.value ? "close" : "clock");
const phaseLabel = computed(() => error.value || t(phases[props.stage.phase] || "session.waiting_for_confirmation"));
</script>

<template>
  <section class="worktree-setup" :data-phase="stage.phase" :aria-label="t('session.preparing_workspace')">
    <header class="worktree-setup-header">
      <span class="worktree-setup-context"><StudioIcon name="folder" /><strong>{{ workspaceName(stage.sourceRoot) }}</strong><span>{{ t("common.isolated_worktree") }}</span></span>
      <h1 ref="heading" tabindex="-1" aria-live="polite">{{ t(headline) }}</h1>
      <p class="worktree-setup-description">{{ t(description) }}</p>
    </header>
    <div class="worktree-setup-phase">
      <span><span v-if="active" class="worktree-spinner" /><StudioIcon v-else :name="icon" /><span>{{ phaseLabel }}</span></span>
      <span v-if="active" class="worktree-setup-elapsed">{{ t("session.waiting_n_s", { n: elapsed }) }}</span>
    </div>
    <p v-if="slow" class="worktree-setup-slow" role="status">{{ t("session.still_waiting_for_creation_you") }}</p>
    <p v-if="stage.phase === 'cleanup_failed'" class="worktree-setup-slow" role="note">{{ t("session.save_your_task_and_attachments") }}</p>
    <div class="worktree-setup-branches"><dl>
      <dt>{{ t("common.source_branch") }}</dt>
      <dd :title="stage.ref"><StudioIcon name="branch" />{{ compactWorkspaceValue(stage.ref.replace(/^refs\/(heads|remotes)\//, ""), 48) }}</dd>
      <dt>{{ t("session.task_branch") }}</dt>
      <dd :title="stage.branch"><StudioIcon name="branch" />{{ stage.branch ? compactWorkspaceValue(stage.branch, 48) : t("session.create_automatically") }}</dd>
    </dl></div>
    <div class="worktree-setup-draft">
      <span class="worktree-setup-label">{{ t("session.your_task") }}</span>
      <p class="worktree-setup-task">{{ task || t("session.see_the_attachments") }}</p>
      <div v-if="files.length > 0" class="worktree-setup-files">
        <span class="worktree-setup-label">{{ t("session.attachments_n", { n: files.length }) }}</span>
        <div v-for="(file, index) in files" :key="`${index}:${file}`">
          <StudioIcon name="file" /><span>{{ file }}</span>
          <small>{{ t(ready ? "session.pending_handoff" : failed || cancelled || retained || uncertain ? "session.retained" : "common.to_send") }}</small>
        </div>
      </div>
    </div>
    <div class="worktree-setup-location"><WorkspacePath :path="failed || cancelled ? stage.sourceRoot : stage.workspaceRoot || stage.sourceRoot" /></div>
    <div v-if="details" class="worktree-setup-details">
      <dl>
        <dt>{{ t("session.operation_id") }}</dt>
        <dd>{{ stage.id }}<CopyButton :text="stage.id" icon-only :label="t('session.copy_operation_id')" /></dd>
        <dt>{{ t("session.current_state") }}</dt>
        <dd>{{ stage.phase }}</dd>
      </dl>
      <pre v-if="stage.detail">{{ stage.detail }}</pre>
    </div>
    <footer class="worktree-setup-actions">
      <button class="worktree-setup-detail-button" data-action="workspace.prepare-details" :aria-expanded="details" @click="details = !details"><StudioIcon name="file" />{{ t(details ? "session.hide_details" : "session.view_details") }}</button>
      <div>
        <button v-if="cancelled || retained || failed" class="btn" data-action="workspace.prepare-edit" :disabled="busy" @click="onEdit">{{ t("session.back_to_editing") }}</button>
        <button v-if="failed || (uncertain && stage.phase !== 'cleanup_failed')" class="btn" data-primary="" data-action="workspace.isolate" :disabled="busy" @click="onRetry"><StudioIcon name="refresh" />{{ t(failed ? "session.retry_creation" : "session.check_again") }}</button>
        <span v-if="ready" class="worktree-setup-handoff"><StudioIcon name="check" />{{ t("session.handing_over_the_task_and") }}</span>
        <button v-else-if="active || stage.phase === 'unreachable'" class="btn" data-action="workspace.prepare-cancel" :disabled="cancelling" @click="onCancel">{{ t(cancelling ? "session.cancelling" : "session.cancel_creation") }}</button>
      </div>
    </footer>
  </section>
</template>
