<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import { ICON, NAV, SETTINGS, pageHome, type Section } from "./prefsnav";
import { arrowTabs } from "./tablist";

const props = defineProps<{
  at: Section;
  onSelect: (section: Section) => void;
  disabled?: boolean;
}>();

/** What a row counts: the settings its page holds, folded-in pages included.
 *  Rows drawn inside another setting's own block are counted by that block, so
 *  counting them here would report the same thing twice. */
const counts = computed(() => {
  const out = {} as Partial<Record<Section, number>>;
  for (const entry of SETTINGS) {
    if (entry.hidden || entry.within) continue;
    const home = pageHome(entry.section);
    out[home] = (out[home] ?? 0) + 1;
  }
  return out;
});

// A page with no row of its own (a folded one opened by name) still needs one
// tab stop in the list, or arrow keys have nowhere to start from.
const stop = computed(() => {
  const ids = NAV.flatMap(([, items]) => items.map(([id]) => id));
  return ids.includes(props.at) ? props.at : ids[0];
});
</script>

<template>
  <nav class="prefs-nav" role="tablist" :aria-label="t('settings.settings_sections')" @keydown="arrowTabs">
    <template v-for="[group, items] in NAV" :key="group">
      <div v-if="group && items.length" class="navsec">{{ t(group) }}</div>
      <button
        v-for="[id, name] in items"
        :id="`prefs-${id}`"
        :key="id"
        role="tab"
        type="button"
        :title="t(name)"
        data-action="settings.section"
        :data-value="id"
        :aria-selected="at === id"
        :aria-current="at === id ? 'page' : undefined"
        :tabindex="stop === id ? 0 : -1"
        :disabled="disabled"
        @click="onSelect(id)"
      >
        <component :is="ICON[id]" />
        <span class="nm">{{ t(name) }}</span>
        <span class="nv" :title="counts[id] ? t('settings.n_items', { n: counts[id] }) : undefined">
          {{ counts[id] || "" }}
        </span>
      </button>
    </template>
  </nav>
</template>
