<script setup lang="ts">
import { ref } from "vue";
import StudioIcon from "./StudioIcon.vue";

/** "+n ⌄" at the end of a strip too narrow for its tabs: the ones it cannot show, to pick from. */
const props = defineProps<{ tabs: { key: string; label: string }[]; onPick: (key: string) => void }>();
const open = ref(false);

const blur = (e: FocusEvent) => {
  if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) open.value = false;
};
const key = (e: KeyboardEvent) => {
  if (e.key === "Escape" && open.value) { e.stopPropagation(); open.value = false; }
};
const pick = (k: string) => { open.value = false; props.onPick(k); };
</script>

<template>
  <span v-if="tabs.length > 0" class="workbench-overflow" data-action-blur="workbench.tab-overflow" data-action-keydown="workbench.tab-overflow"
    @blur="blur" @keydown="key">
    <button type="button" class="workbench-overflow-btn" data-action="workbench.tab-overflow" aria-haspopup="menu" :aria-expanded="open" @click="open = !open">
      +{{ tabs.length }}
      <StudioIcon name="down" />
    </button>
    <div v-if="open" class="workbench-overflow-menu" role="menu">
      <button v-for="tab in tabs" :key="tab.key" type="button" role="menuitem" data-action="workbench.tab" :data-target="tab.key" @click="pick(tab.key)">
        {{ tab.label }}
      </button>
    </div>
  </span>
</template>