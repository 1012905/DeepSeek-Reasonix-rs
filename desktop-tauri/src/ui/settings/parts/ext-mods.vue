<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, say } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type { AgentPort, ModState, ModStatus, ModsSettings, ModsSettingsEdit } from "../../../port/port";
import Switch from "../../Switch.vue";
import type { StudioIconName } from "../../StudioIcon.vue";
import StudioIcon from "../../StudioIcon.vue";

const BUDGET_MAX = 55000;

// The words and the mark for each state: a state never rests on colour alone.
const STATE: Record<ModState, { icon: StudioIconName; word: string }> = {
  inactive: { icon: "inactive", word: "ext.not_running" },
  starting: { icon: "loading", word: "ext.starting" },
  running: { icon: "running", word: "common.running" },
  restarting: { icon: "refresh", word: "ext.restarting" },
  disabled: { icon: "disabled", word: "common.suspended" },
};

// Modules an installed package ships, run in a process of their own with the
// user's full trust. The switch is the only authorisation to run any of them, so
// it says what that means in plain words; a client that may not edit it sees it
// read-only rather than failing on a click.
const props = defineProps<{ port: AgentPort }>();

const settings = ref<ModsSettings | null>(null);
const mods = ref<ModStatus[] | null>(null);
const draft = ref("");
const busy = ref(false);
const readOnly = ref(false);
const error = ref("");

const adopt = (s: ModsSettings) => {
  settings.value = s;
  draft.value = String(s.hook_budget_ms);
};

watch(() => props.port, (port) => {
  let live = true;
  error.value = "";
  readOnly.value = false;
  port.modsSettings()
    .then((s) => { if (live) adopt(s); })
    .catch((e) => { if (live) error.value = reason(e); });
  port.mods()
    .then((m) => { if (live) mods.value = m; })
    .catch(() => { if (live) mods.value = []; });
  return () => { live = false; };
}, { immediate: true });

const save = async (edit: ModsSettingsEdit) => {
  busy.value = true;
  error.value = "";
  try {
    adopt(await props.port.saveModsSettings(edit));
    mods.value = await props.port.mods();
  } catch (e) {
    if (e instanceof HttpError && e.reason?.code === "mods.editing_disabled") readOnly.value = true;
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};

// A hook's own budget is written in milliseconds and nothing else; a value past
// the cap is refused by the kernel, so it is caught before the round trip.
const commit = () => {
  const now = settings.value;
  if (!now) return;
  const n = Number(draft.value);
  if (!Number.isInteger(n) || n < 0 || n > BUDGET_MAX) {
    error.value = t("common.enter_a_whole_number_from", { min: 0, max: BUDGET_MAX });
    return;
  }
  if (n !== now.hook_budget_ms) void save({ hook_budget_ms: n });
};

const locked = computed(() => busy.value || readOnly.value);
</script>

<template>
  <div v-if="!settings" class="empty">{{ error || t("ext.the_mod_settings_could_not") }}</div>
  <div v-else class="box">
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("ext.run_claude_code_mods") }}</span>
        <span class="ds">{{ t("ext.off_by_default_when_on") }}</span>
      </span>
      <Switch
        data-action="mods.enabled"
        :on="settings.enabled"
        :busy="locked"
        :label="t('ext.run_claude_code_mods')"
        :on-click="() => void save({ enabled: !settings!.enabled })"
      />
    </div>
    <div class="lrow threshold-row">
      <span class="tx">
        <span class="lb">{{ t("ext.time_limit_for_one_hook") }}</span>
        <span class="ds">{{ t("ext.past_this_time_the_mods", { max: BUDGET_MAX }) }}</span>
      </span>
      <div class="threshold-control">
        <span class="unit-field">
          <input
            v-model="draft"
            type="text"
            inputmode="numeric"
            data-action-keydown="mods.budget"
            data-action-change="mods.budget"
            :aria-label="t('ext.time_limit_for_one_hook')"
            :disabled="locked"
            @input="draft = draft.replace(/\D/g, '')"
            @blur="commit"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
          <i>{{ t("ext.ms") }}</i>
        </span>
      </div>
    </div>
    <div v-if="readOnly" class="peernote" role="status">{{ t("common.these_settings_are_read_only") }}</div>
    <ul class="modlist" :aria-label="t('ext.installed_mods')">
      <li v-for="m in mods ?? []" :key="m.plugin" :data-state="m.state">
        <div class="modhead">
          <b>{{ m.plugin }}</b>
          <span class="modstate"><StudioIcon :name="STATE[m.state].icon" /> {{ t(STATE[m.state].word) }}</span>
        </div>
        <div v-if="m.code" class="ds">{{ say({ code: m.code, error: m.detail }) }}</div>
        <div v-if="m.restarts > 0" class="ds">{{ t("ext.restarted_n_times", { n: m.restarts }) }}</div>
        <div v-if="m.events?.length" class="ds">{{ t("ext.events_it_registered") }}：{{ m.events.join("、") }}</div>
        <div v-if="m.unserved?.length" class="ds">{{ t("ext.events_this_app_never_raises_2") }}：{{ m.unserved.join("、") }}</div>
      </li>
      <li v-if="mods && mods.length === 0" class="ds">{{ t("ext.no_installed_package_carries_a") }}</li>
    </ul>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
