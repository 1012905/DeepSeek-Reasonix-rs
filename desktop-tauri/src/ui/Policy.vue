<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";
import { reason } from "../i18n/kernel";
import type { AgentPort, ApprovalMode, SessionStatus } from "../port/port";
import { useDismiss } from "./dismiss";
import { APPROVALS, approvalName } from "./approvals";

// Permission is one boundary, not a catch-all for every way a turn can run.
// Work strategy and model effort live in their own controls beside this one.
const props = defineProps<{
  port: AgentPort;
  status: SessionStatus | null;
  onChanged: () => void;
  onBoundary?: () => void;
}>();

const open = ref(false);
const asking = ref<ApprovalMode | "">("");
const refused = ref("");
const wrap = ref<HTMLDivElement | null>(null);
useDismiss(open, wrap, () => { open.value = false; });

const apv = computed(() => props.status?.toolApprovalMode ?? "ask");
const label = computed(() => approvalName(apv.value));
const apply = (value: ApprovalMode) => {
  if (asking.value) return;
  asking.value = value;
  refused.value = "";
  props.port.setApprovalMode(value)
    .then(() => props.onChanged())
    .catch((error: unknown) => { refused.value = reason(error); })
    .finally(() => { asking.value = ""; });
};
</script>

<template>
  <div v-if="status" class="picker policy" ref="wrap" :data-quiet="apv === 'ask' ? '' : undefined">
    <button
      class="mode plain"
      data-action="chrome.policy"
      :data-risk="apv === 'yolo' ? '' : undefined"
      :aria-expanded="open"
      :aria-label="`${t('safety.execution_permissions')}：${label}`"
      :title="`${t('safety.execution_permissions')}：${label}`"
      @click="open = !open"
    >
      <span class="ic" aria-hidden="true">
        <i v-if="apv === 'yolo'" class="polwarn"><StudioIcon name="warning" /></i>
        <StudioIcon v-else name="shield" />
      </span>
      <span :class="apv === 'yolo' ? 'lb polrisk' : 'lb'" :data-display="label">{{ label }}</span>
      <StudioIcon name="down" class="studio-control-chevron" />
    </button>
    <div class="menu modemenu polmenu composer-menu" role="group" :aria-label="t('safety.execution_permissions')" :hidden="!open">
      <div class="studio-policy-head"><b>{{ t("safety.execution_permissions") }}</b><small>{{ t("common.current_task") }}</small></div>
      <div class="studio-policy-primary">
        <div v-for="row in APPROVALS" :key="row[0]">
          <div v-if="row[0] === 'yolo'" class="studio-policy-risk">{{ t("common.high_risk") }}</div>
          <button
            :class="['studio-permission-option', row[0] === 'yolo' ? 'studio-permission-yolo' : '']"
            data-action="tool-approval.mode"
            :data-value="row[0]"
            :data-on="apv === row[0] ? '' : undefined"
            :data-pending="asking === row[0] ? '' : undefined"
            :aria-pressed="apv === row[0]"
            :disabled="!!asking"
            @click="apply(row[0])"
          >
            <span><b>{{ t(row[1]) }}</b><small>{{ t(row[2]) }}</small></span>
            <StudioIcon v-if="apv === row[0]" name="check" class="studio-permission-check" />
          </button>
        </div>
        <div v-if="refused" class="note bad studio-policy-error" role="alert">{{ refused }}</div>
      </div>
      <div class="studio-policy-note">{{ t("safety.approvals_decide_when_to_ask") }}</div>
      <button class="studio-policy-boundary" data-action="settings.section" data-value="tools:sandbox" :disabled="!onBoundary" @click="onBoundary?.()">
        <span><b>{{ t("safety.sandbox_and_runtime_boundary") }}</b><small>{{ t("safety.view_current_isolation_and_network") }}</small></span>
        <span aria-hidden="true"><StudioIcon name="chevron" /></span>
      </button>
    </div>
  </div>
</template>
