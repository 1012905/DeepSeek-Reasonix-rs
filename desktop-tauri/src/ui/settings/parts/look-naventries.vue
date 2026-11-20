<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import { useExternal } from "../../../state/external";
import { navLayout, onNavLayoutChange, ordered, resetNavLayout, setNavLayout } from "../../../state/navEntries";
import { DESTINATIONS } from "../../navdest";
import StudioIcon from "../../StudioIcon.vue";
import Switch from "../../Switch.vue";
import Group from "../../Group.vue";

const NAMES = new Map(DESTINATIONS.map(([mark, label]) => [mark as string, label]));
const KEYS = DESTINATIONS.map(([mark]) => mark as string);

const layout = useExternal(onNavLayoutChange, navLayout);
// Every destination in this machine's order, hidden ones included: a hidden
// entry still has a place, or it could never be brought back.
const list = computed(() => ordered(KEYS, layout.value));
const hidden = computed(() => new Set(layout.value.hidden));
const changed = computed(() => layout.value.order.length > 0 || layout.value.hidden.length > 0);

const nameOf = (key: string) => t(NAMES.get(key) ?? key);

const toggle = (key: string) =>
  setNavLayout({
    order: list.value,
    hidden: hidden.value.has(key) ? layout.value.hidden.filter((k) => k !== key) : [...layout.value.hidden, key],
  });

// Reordering writes the order it produced, not a delta: a stored list is the
// only thing the sidebar reads.
const move = (key: string, by: -1 | 1) => {
  const at = list.value.indexOf(key);
  const next = [...list.value];
  const to = at + by;
  if (at < 0 || to < 0 || to >= next.length) return;
  [next[at], next[to]] = [next[to]!, next[at]!];
  setNavLayout({ order: next, hidden: layout.value.hidden });
};
</script>

<template>
  <Group id="naventries" :title="t('settings.navigation_entries')" :hint="t('settings.which_entries_the_left_rail_shows')">
    <div v-for="(key, i) in list" :key="key" class="lrow">
      <span class="tx"><span class="lb">{{ nameOf(key) }}</span></span>
      <button
        type="button"
        class="btn sm"
        data-action="appearance.nav-entry-move"
        :data-value="`${key}:up`"
        :disabled="i === 0"
        :aria-label="t('settings.move_up_name', { name: nameOf(key) })"
        @click="move(key, -1)"
      >
        <StudioIcon name="arrow" />
      </button>
      <button
        type="button"
        class="btn sm"
        data-action="appearance.nav-entry-move"
        :data-value="`${key}:down`"
        :disabled="i === list.length - 1"
        :aria-label="t('settings.move_down_name', { name: nameOf(key) })"
        @click="move(key, 1)"
      >
        <StudioIcon name="arrowDown" />
      </button>
      <Switch
        data-action="appearance.nav-entry-visible"
        :data-value="key"
        :on="!hidden.has(key)"
        :label="t('settings.show_name', { name: nameOf(key) })"
        :on-click="() => toggle(key)"
      />
    </div>
    <button type="button" class="btn sm" data-action="appearance.nav-entries-reset" :disabled="!changed" @click="resetNavLayout">
      {{ t("settings.reset_arrangement") }}
    </button>
  </Group>
</template>
