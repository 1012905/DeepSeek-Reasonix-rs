<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { liveStreamBudget, MAX_LIVE_STREAM_BUDGET, onLiveStreamBudgetChange, setLiveStreamBudget } from "../state/prefs";
import Group from "./Group.vue";

// How many open session tabs may keep a live connection at once. Empty is
// automatic. A browser holds six connections to one address and the session list
// takes one, so the window keeps a few live and reads the rest when they are shown.
const chosen = useExternal(onLiveStreamBudgetChange, liveStreamBudget);
const draft = ref(chosen.value === null ? "" : String(chosen.value));
watch(chosen, (v) => (draft.value = v === null ? "" : String(v)));
const commit = () => {
  setLiveStreamBudget(draft.value === "" ? null : Number(draft.value));
  const now = liveStreamBudget();
  draft.value = now === null ? "" : String(now);
};
const typed = (e: Event) => (draft.value = (e.target as HTMLInputElement).value.replace(/\D/g, ""));
const keyed = (e: KeyboardEvent) => {
  if (e.key !== "Enter") return;
  commit();
  (e.currentTarget as HTMLInputElement).blur();
};
</script>

<template>
  <Group
    id="live-stream-budget"
    :title="t('settings.session_tabs_kept_live')"
    :hint="t('settings.empty_means_automatic_a_browser')"
  >
    <div class="lrow">
      <span class="tx">
        <label class="lb" for="live-stream-budget-input">{{ t("settings.tabs_with_a_live_connection") }}</label>
        <span class="ds">{{ t("settings.empty_is_automatic_1_to", { max: MAX_LIVE_STREAM_BUDGET }) }}</span>
      </span>
      <span class="unit-field">
        <input
          id="live-stream-budget-input"
          type="text"
          inputmode="numeric"
          :placeholder="t('common.auto')"
          :value="draft"
          data-action-change="appearance.live-stream-budget"
          data-action-keydown="appearance.live-stream-budget"
          @input="typed"
          @blur="commit"
          @keydown="keyed"
        />
        <i>{{ t("settings.tabs") }}</i>
      </span>
    </div>
  </Group>
</template>