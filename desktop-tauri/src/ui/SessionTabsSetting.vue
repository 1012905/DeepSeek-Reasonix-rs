<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onShowsSessionTabsChange, setShowsSessionTabs, showsSessionTabs } from "../state/prefs";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const sessionTabs = useExternal(onShowsSessionTabsChange, showsSessionTabs);
</script>

<template>
  <Group id="session-tabs" :inline="true" :title="t('session.session_tabs')" :hint="t('session.show_tabs_at_the_top')">
    <Switch data-action="appearance.session-tabs" :on="sessionTabs" :label="t('session.show_session_tabs')"
      :on-click="() => setShowsSessionTabs(!sessionTabs)" />
  </Group>
</template>