<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onSessionQuickActionsChange, setShowsSessionQuickActions, showsSessionQuickActions } from "../state/sessionQuick";
import Group from "./Group.vue";

const on = useExternal(onSessionQuickActionsChange, showsSessionQuickActions);
</script>

<template>
  <Group id="session-quick-actions" :inline="true" :title="t('session.session_row_buttons')" :hint="t('settings.buttons_shown_on_row_hover')">
    <div class="seg" data-text="true" role="group" :aria-label="t('session.session_row_buttons')">
      <button data-action="appearance.session-quick-actions" data-value="off" :aria-pressed="!on" @click="setShowsSessionQuickActions(false)">{{ t("common.close") }}</button>
      <button data-action="appearance.session-quick-actions" data-value="on" :aria-pressed="on" @click="setShowsSessionQuickActions(true)">{{ t("common.show") }}</button>
    </div>
  </Group>
</template>