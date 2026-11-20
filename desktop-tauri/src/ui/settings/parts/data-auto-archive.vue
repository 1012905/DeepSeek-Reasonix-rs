<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { AutoArchiveSettings } from "../../../port/boundary";
import SettingRow from "../../SettingRow.vue";
import Switch from "../../Switch.vue";

// Archiving hides a conversation from the everyday list and nothing else, so the
// switch ships off and the number only matters once it is on.
const LIMIT = 3650;

const props = defineProps<{ port: AgentPort }>();

const state = ref<AutoArchiveSettings | null>(null);
const days = ref("");
const busy = ref(false);
const error = ref("");

const adopt = (s: AutoArchiveSettings) => {
  state.value = s;
  days.value = String(s.days);
};

watch(() => props.port, (port) => {
  let live = true;
  port.autoArchive().then((s) => { if (live) adopt(s); }).catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

async function save(enabled: boolean, n: number): Promise<void> {
  busy.value = true;
  error.value = "";
  try {
    adopt(await props.port.saveAutoArchive({ enabled, days: n }));
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// A number outside the range is refused here rather than sent: the kernel would
// answer with a refusal, and the field would already have lost what was typed.
function commit(): void {
  const held = state.value;
  if (!held) return;
  const n = Number(days.value);
  if (!Number.isInteger(n) || n < 1 || n > LIMIT) {
    error.value = t("common.enter_a_whole_number_from_2", { n: LIMIT });
    days.value = String(held.days);
    return;
  }
  error.value = "";
  if (n !== held.days) void save(held.enabled, n);
}

const toggle = () => {
  const held = state.value;
  if (held && !busy.value) void save(!held.enabled, held.days);
};

const digits = (e: Event) => { days.value = (e.target as HTMLInputElement).value.replace(/\D/g, ""); };
const enter = (e: KeyboardEvent) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); };
</script>

<template>
  <SettingRow
    id="auto-archive"
    :title="t('data.auto_archive_inactive_conversations')"
    :desc="t('settings.days_before_archiving')"
    :more="t('data.archiving_only_moves_a_conversation')"
  >
    <span class="unit-field">
      <input
        type="text"
        inputmode="numeric"
        :aria-label="t('data.idle_days')"
        data-action-change="auto-archive.days"
        :value="days"
        :disabled="busy || !state"
        @input="digits"
        @blur="commit"
        @keydown="enter"
      />
      <i>{{ t("data.days") }}</i>
    </span>
    <Switch
      data-action="auto-archive.enabled"
      :on="!!state?.enabled"
      :busy="busy || !state"
      :label="t('data.archive_conversations_left_idle')"
      :on-click="toggle"
    />
  </SettingRow>
  <div v-if="error" class="why" role="alert">{{ error }}</div>
  <div v-else-if="!state" class="empty">{{ t("data.could_not_read_the_auto") }}</div>
</template>
