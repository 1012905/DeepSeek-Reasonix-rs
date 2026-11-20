<script lang="ts">
import { t } from "../i18n";
import type { ProviderRemoval } from "../port/port";

const ROLE_LABEL: Record<string, () => string> = {
  default: () => t("common.default_model"),
  planner: () => t("common.plan"),
  subagent: () => t("common.subagents"),
  vision: () => t("models.vision"),
  guardian: () => t("models.review"),
  recovery: () => t("models.recovery_review"),
  triage: () => t("models.small_classifications"),
  decision: () => t("models.decision"),
  advisor: () => t("models.advisor"),
};

export function roleLabel(id: string): string {
  const [role, skill] = id.split(":", 2);
  const label = ROLE_LABEL[role]?.() ?? role;
  return skill ? `${label} · ${skill}` : label;
}

export const removalChangedRoles = (r: ProviderRemoval) => r.moved.length > 0 || r.cleared.length > 0;

export interface Removal {
  key: string;
  name: string;
  why: string;
  report: ProviderRemoval | null;
}
</script>

<script setup lang="ts">
import { computed } from "vue";

// Where a removal's outcome shows once the service it concerns has no detail
// to carry it: the roles it handed on or switched off, or a failure that came
// after the removal was already written.
const props = defineProps<{ removal: Removal }>();
const moved = computed(() => props.removal.report?.moved.map(roleLabel).join("、") ?? "");
const cleared = computed(() => props.removal.report?.cleared.map(roleLabel).join("、") ?? "");
</script>

<template>
  <div class="find" data-lvl="warn" :role="removal.why ? 'alert' : 'status'">
    <span class="t">{{ removal.why ? t("models.removing_name_did_not_fully", { name: removal.name }) : t("models.removed_name", { name: removal.name }) }}</span>
    <span v-if="removal.why" class="why">{{ removal.why }}</span>
    <span v-if="moved" class="why">{{ t("models.now_using_to_for_roles", { to: removal.report?.movedTo ?? "", roles: moved }) }}</span>
    <span v-if="cleared" class="why">{{ t("models.cleared_so_these_are_off", { roles: cleared }) }}</span>
  </div>
</template>