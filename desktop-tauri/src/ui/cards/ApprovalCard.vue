<script lang="ts">
export type { PlanAction } from "../../port/session";
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { ApprovalVerdict, PlanAction } from "../../port/session";
import type { Item } from "../../state/session";
import { useViewer } from "../../state/viewer";
import { decisionSource } from "../source";
import RememberChoice, { RememberWhere, type Remembered } from "./RememberChoice.vue";
import { useFolded } from "../usefold";
import { isPlanGate } from "../planLive";
import PlanCard from "./PlanCard.vue";
import Sym from "../Sym.vue";

// Widening a delegated run's write confinement. Not a tool the run calls: the
// answer moves the fence rather than performing anything, so the card says that
// instead of "about to run extend_write_paths".
const FENCE_TOOL = "extend_write_paths";
// The model asking to change one of the user's settings. The card is worded by
// the kernel; the elevated one offers no session grant, which `allowsSession` already says.
const SETTING_TOOL = "setting_change";
const SETTING_ELEVATED_TOOL = "setting_change_elevated";
// A call that repeats text from outside; the card says so instead of showing the tool id.
const ORIGIN_TOOL = "origin_check";

// Why no posture answers for this call. The kernel names the class and writes
// the sentence it gave the model; a reader gets that class in their own
// language, and an unknown one falls back to what the model was told.
const EXPLICIT_APPROVAL: Record<string, string> = {
  dynamic_bash: "chat.this_command_runs_another_command",
  browser_credential: "chat.this_step_types_a_password",
  computer_use: "chat.this_reads_or_operates_another",
  computer_front: "chat.this_operates_another_application_on",
  origin_text: "chat.text_in_this_call_is",
  computer_pointer: "chat.this_step_takes_the_pointer",
};

const TASK_PREVIEW = 280;

const props = defineProps<{
  item: Extract<Item, { t: "approval" }>;
  onApprove: (itemId: string, id: string, v: ApprovalVerdict) => Promise<void>;
  onFullAccess: (itemId: string) => Promise<void>;
  onPlan: (itemId: string, id: string, action: PlanAction) => Promise<void>;
}>();

const sealed = computed(() => props.item.verdict !== undefined);
const viewer = useViewer();
const settledBy = computed(() => decisionSource(props.item.by, viewer.value));
const submitting = ref<ApprovalVerdict | "">("");
const confirmFullAccess = ref(false);
const moreOpen = ref(false);
const phone = useFolded("phone");
const switchingFullAccess = ref(false);
const remembering = computed<Remembered[]>(() => [
  ...(props.item.a.allowsSession ? ["session" as const] : []),
  ...(props.item.a.allowsPersist ? ["always" as const] : []),
]);
// Full access cannot answer the origin question, so the card does not offer it.
const explicit = computed(() =>
  props.item.a.tool !== ORIGIN_TOOL && (!!props.item.a.reasonCode || !!props.item.a.reason?.toLowerCase().includes("nested or indirect shell execution")),
);
const isSetting = computed(() => props.item.a.tool === SETTING_TOOL || props.item.a.tool === SETTING_ELEVATED_TOOL);

const title = computed(() => {
  if (props.item.a.tool === FENCE_TOOL) return t("chat.asking_to_widen_what_it");
  if (isSetting.value) return t("chat.asks_to_change_a_setting");
  return t("chat.permission_to_run");
});

const toolLabel = computed(() => {
  if (props.item.a.tool === FENCE_TOOL) return t("chat.a_file_outside_this_subtasks");
  if (isSetting.value) return t("common.settings");
  if (props.item.a.tool === ORIGIN_TOOL) return t("chat.text_from_outside");
  return props.item.a.tool;
});

const scopeUnattended = computed(() => props.item.a.scope === "unattended_attempts");
const scopeN = computed(() => {
  const n = props.item.a.input?.n;
  return typeof n === "number" ? n : undefined;
});
const scopeTask = computed(() => {
  const p = props.item.a.input?.prompt;
  return typeof p === "string" ? p.trim() : "";
});
const taskPreview = computed(() => (scopeTask.value.length > TASK_PREVIEW ? scopeTask.value.slice(0, TASK_PREVIEW) + "…" : scopeTask.value));
const scopePaths = computed(() => {
  const paths = props.item.a.input?.paths;
  return Array.isArray(paths) ? paths.filter((p): p is string => typeof p === "string") : [];
});

const reasonText = computed(() => {
  const a = props.item.a;
  if (a.reasonCode && EXPLICIT_APPROVAL[a.reasonCode]) return t(EXPLICIT_APPROVAL[a.reasonCode]!);
  // Older kernels only sent the English sentence. Keep that compatibility
  // path readable instead of exposing an implementation message in a Chinese UI.
  if (a.reason?.toLowerCase().includes("nested or indirect shell execution")) return t(EXPLICIT_APPROVAL.dynamic_bash!);
  return a.reason || "";
});

const sealedWords = computed<[string, string]>(() => {
  switch (props.item.verdict) {
    case "session": return [t("chat.allowed_this_kind_will_not"), t("chat.the_kernel_records_it_for")];
    case "always": return [t("chat.allowed_recorded_as_a_rule"), t("chat.written_to_the_config_later")];
    case "yolo": return [t("chat.full_access_enabled"), t("chat.future_tools_will_not_ask")];
    case "deny": return [t("chat.denied"), t("chat.the_agent_takes_the_refusal")];
    case "unknown": return [t("chat.handled_in_another_window"), t("chat.refer_to_the_latest_run")];
    default: return [t("chat.allowed_once"), t("chat.the_same_operation_will_ask")];
  }
});

const decide = async (verdict: ApprovalVerdict) => {
  if (submitting.value) return;
  submitting.value = verdict;
  try {
    await props.onApprove(props.item.id, props.item.a.id, verdict);
  } finally {
    submitting.value = "";
  }
};

const useFullAccess = async () => {
  if (switchingFullAccess.value) return;
  switchingFullAccess.value = true;
  try {
    await props.onFullAccess(props.item.id);
  } finally {
    switchingFullAccess.value = false;
  }
};
</script>

<template>
  <PlanCard v-if="isPlanGate(item.a)" :item="item" :on-plan="onPlan" />
  <!-- Approval borrows the "write" category: asking a question is not the
       same act as being handed permission to change a file. -->
  <div v-else class="call" data-k="write" :data-approval="sealed ? item.verdict : 'pending'" :data-prompt="sealed ? 'settled' : 'pending'">
    <div class="g">
      <Sym icon="shield" />
    </div>
    <div class="c">
      <div class="hl">
        <span class="nm">{{ title }}</span>
        <span class="tag" :data-pending="sealed ? undefined : ''">{{ sealed ? t("chat.handled") : t("chat.needs_you") }}</span>
      </div>
      <div class="out">
        <div class="apv" :data-sealed="sealed ? item.verdict : undefined" :aria-busy="!!submitting">
          <div class="apv-hd" :data-whole="item.a.tool === ORIGIN_TOOL ? '' : undefined">
            <span class="tool">{{ toolLabel }}</span>
            <span class="sub" :title="item.a.subject">{{ item.a.subject }}</span>
          </div>
          <template v-if="item.a.scope">
            <template v-if="scopeUnattended">
              <div class="apv-dt" :data-scope="item.a.scope">
                <span class="apv-dt-label">{{ t("chat.approving_also") }}</span>
                <span>{{ scopeN ? t("chat.starts_n_attempts_in_parallel", { n: scopeN }) : t("chat.starts_several_attempts_in_parallel") }}{{ t("chat.each_running_unattended_in_its") }}</span>
              </div>
              <div v-if="scopeTask" class="apv-dt">
                <span class="apv-dt-label">{{ t("chat.the_attempts_task") }}</span>
                <span :title="scopeTask">{{ taskPreview }}</span>
              </div>
            </template>
            <div v-else-if="item.a.scope === 'open_projects'" class="apv-dt" :data-scope="item.a.scope">
              <span class="apv-dt-label">{{ t("chat.approving_also") }}</span>
              <span>
                {{ t("chat.adds_these_n_folders_to", { n: scopePaths.length }) }}
                <ul class="apv-paths"><li v-for="p in scopePaths" :key="p"><code :title="p">{{ p }}</code></li></ul>
              </span>
            </div>
            <div v-else class="apv-dt" :data-scope="item.a.scope">
              <span class="apv-dt-label">{{ t("chat.approving_also") }}</span>
              <span>{{ t("chat.this_approval_covers_more_than") }}</span>
            </div>
          </template>
          <div v-if="reasonText" class="apv-dt">
            <span class="apv-dt-label">{{ t("chat.why_confirmation_is_needed") }}</span>
            <span :style="isSetting ? { whiteSpace: 'pre-line' } : undefined">{{ reasonText }}</span>
          </div>
          <div v-if="item.a.tool === SETTING_ELEVATED_TOOL" class="apv-dt" data-setting="elevated">
            <span class="apv-dt-label">{{ t("chat.asked_every_time") }}</span>
            <span>{{ t("chat.this_setting_loosens_what_the") }}</span>
          </div>
          <div v-if="!sealed" class="apv-ft" :data-more="moreOpen ? '' : undefined">
            <button class="btn" data-primary="" data-action="decision.tool" :data-target="item.a.id" data-value="once"
              :disabled="!!submitting" @click="decide('once')">
              {{ submitting === "once" ? t("common.submitting") : t("chat.allow_once") }}
            </button>
            <!-- The remembering choices read quieter than "once". The kernel
                 names which grants it honours here; drawing another would
                 promise what it cannot do. -->
            <RememberChoice :options="remembering" :target="item.a.id" :busy="submitting"
              :disabled="!!submitting" :on-choose="decide" />
            <button class="btn" data-deny="" data-action="decision.tool" :data-target="item.a.id" data-value="deny"
              :disabled="!!submitting" @click="decide('deny')">
              {{ t("common.refuse") }}
            </button>
            <button v-if="explicit" class="btn" data-weak="" data-yolo="" data-action="decision.full-access"
              :disabled="!!submitting" @click="confirmFullAccess = true">
              {{ t("chat.switch_to_full_access") }}
            </button>
            <button v-if="phone && (remembering.length > 0 || explicit)" type="button" class="apv-more" data-action="decision.more"
              :aria-expanded="moreOpen" @click="moreOpen = !moreOpen">
              {{ t("chat.more_options") }}
            </button>
          </div>
          <RememberWhere v-if="!sealed" :options="remembering" :target="item.a.id" :tool="item.a.tool" />
          <div v-if="!sealed && confirmFullAccess" class="apv-yolo-confirm" role="alertdialog" :aria-label="t('chat.confirm_full_access_2')">
            <div><b>{{ t("chat.full_access_skips_future_tool") }}</b><span>{{ t("chat.deny_rules_and_sandbox_boundaries") }}</span></div>
            <div>
              <button class="btn" :disabled="switchingFullAccess" @click="confirmFullAccess = false">{{ t("common.back") }}</button>
              <button class="btn" data-yolo-confirm="" :disabled="switchingFullAccess" @click="useFullAccess()">
                {{ switchingFullAccess ? t("chat.switching") : t("chat.confirm_full_access") }}
              </button>
            </div>
          </div>
          <div v-if="sealed" class="apv-done">
            <b>{{ sealedWords[0] }}</b>{{ sealedWords[1] }}
            <span v-if="settledBy" class="apv-by">{{ settledBy }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
