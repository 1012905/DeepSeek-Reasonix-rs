<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { NAV_RAIL_MODES, navRailMode, onNavRailChange, setNavRailMode, type NavRailMode } from "../state/prefs";
import Group from "./Group.vue";

const LABEL: Record<NavRailMode, string> = { on: "settings.always", collapsed: "settings.only_when_the_sidebar_is", off: "settings.never" };

const mode = useExternal(onNavRailChange, navRailMode);
</script>

<template>
  <Group id="navrail" inline :title="t('settings.icon_rail')" :hint="t('settings.which_entries_the_left_rail_shows')">
    <div class="seg" data-text role="group" :aria-label="t('settings.icon_rail')">
      <button v-for="m in NAV_RAIL_MODES" :key="m" data-action="appearance.nav-rail" :data-value="m" :aria-pressed="mode === m" @click="setNavRailMode(m)">
        {{ t(LABEL[m]) }}
      </button>
    </div>
    <p class="note">{{ t("settings.with_it_off_or_hidden") }}</p>
  </Group>
</template>