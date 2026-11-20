<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onOpenSessionLimitChange, openSessionLimit, setOpenSessionLimit } from "../state/prefs";
import Group from "./Group.vue";

// How many sessions the window keeps open as tabs. Zero is the default and means
// none is ever folded away on its own; a number folds the one idle longest once
// the window holds that many. Typing is limited to digits, and what is stored is
// read back from the preference, so the box always shows what will happen.
const limit = useExternal(onOpenSessionLimitChange, openSessionLimit);
const draft = ref(String(limit.value));
watch(limit, (v) => { draft.value = String(v); });

function commit() {
  setOpenSessionLimit(draft.value === "" ? 0 : Number(draft.value));
  draft.value = String(openSessionLimit());
}

function onInput(e: Event) {
  const el = e.target as HTMLInputElement;
  const next = el.value.replace(/\D/g, "");
  draft.value = next;
  el.value = next;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Enter") return;
  commit();
  (e.currentTarget as HTMLInputElement).blur();
}
</script>

<template>
  <Group
    id="open-session-limit"
    :title="t('settings.session_tabs_kept_open')"
    :hint="t('settings.default_0_nothing_is_folded')"
  >
    <div class="lrow">
      <span class="tx">
        <label class="lb" for="open-session-limit-input">{{ t("settings.limit_on_session_tabs") }}</label>
        <span class="ds">{{ t("settings.n_0_means_no_limit") }}</span>
      </span>
      <span class="unit-field">
        <input
          id="open-session-limit-input"
          type="text"
          inputmode="numeric"
          :value="draft"
          data-action-change="appearance.open-session-limit"
          data-action-keydown="appearance.open-session-limit"
          @input="onInput"
          @blur="commit"
          @keydown="onKeydown"
        />
        <i>{{ t("settings.tabs") }}</i>
      </span>
    </div>
  </Group>
</template>