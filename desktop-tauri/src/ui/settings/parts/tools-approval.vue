<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, ApprovalMode, SessionStatus } from "../../../port/port";
import { APPROVALS, approvalName, approvalNote } from "../../approvals";
import { SegChoice } from "../../SettingRow.vue";

// The same one table the composer menu and the shelf read: an approval mode
// named in three places cannot be spelled three ways. Ordered strictest first.
const props = defineProps<{ port: AgentPort; status: SessionStatus | null; onChanged: () => void }>();

const asking = ref<ApprovalMode | "">("");
const refused = ref("");
const mode = computed<ApprovalMode>(() => props.status?.toolApprovalMode ?? "ask");

// Kept in step with the session the shell hands down, so a mode changed from the
// composer shows here without this panel owning a second copy of the answer.
const shown = ref<ApprovalMode>(mode.value);
watch(mode, (next) => { shown.value = next; }, { immediate: true });

const choice = computed(() => SegChoice<ApprovalMode>({
  label: t("settings.tool_approvals"),
  value: shown.value,
  options: APPROVALS.map(([id, name]) => [id, t(name)] as const),
  disabled: !!asking.value,
  action: "tool-approval.mode",
  onPick: (next) => void apply(next),
}));

const apply = async (next: ApprovalMode) => {
  if (asking.value) return;
  asking.value = next;
  refused.value = "";
  try {
    await props.port.setApprovalMode(next);
    shown.value = next;
    props.onChanged();
  } catch (e) {
    refused.value = reason(e);
  } finally {
    asking.value = "";
  }
};

const note = computed(() => (shown.value === "yolo" ? approvalNote(shown.value) : ""));
const name = computed(() => approvalName(shown.value));
</script>

<template>
  <div class="lrow">
    <span class="tx">
      <span class="lb">{{ t("safety.ask_before_writing_or_running") }}</span>
      <span class="ds">{{ t("safety.approvals_decide_when_to_ask") }}</span>
    </span>
    <component :is="choice" />
    <span class="sc">{{ name }}</span>
  </div>
  <p v-if="note" class="note" role="status">{{ note }}</p>
  <div v-if="refused" class="why" role="alert">{{ refused }}</div>
</template>
