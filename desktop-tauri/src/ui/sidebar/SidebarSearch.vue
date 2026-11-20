<script lang="ts">
/** The box at the head of the column. It is controlled: the caller owns the
 *  word, because the same word narrows the project list, opens folded projects
 *  and drives the content search below — three readers of one value. */
export interface SidebarSearchProps {
  value: string;
  /** The combination this box prints, so the label it shows is not a guess. */
  shortcut: string;
}

export function shortcutLabel(): string {
  const mac = typeof navigator !== "undefined" && /mac|iphone|ipad|ipod/i.test(navigator.userAgent);
  return mac ? "⌘K" : "Ctrl K";
}
</script>

<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<SidebarSearchProps>();
const emit = defineEmits<{ (e: "query", value: string): void; (e: "palette"): void }>();

const box = ref<HTMLInputElement | null>(null);

function onInput(event: Event) {
  emit("query", (event.target as HTMLInputElement).value);
}

function clear() {
  emit("query", "");
  box.value?.focus();
}

function onKey(event: KeyboardEvent) {
  // Escape unwinds the word first: clearing it here must not also close
  // whatever the column is drawn inside.
  if (event.key !== "Escape" || !props.value) return;
  event.preventDefault();
  event.stopPropagation();
  clear();
}

defineExpose({ focus: () => { box.value?.focus(); box.value?.select(); }, clear });
</script>

<template>
  <div class="wsfind" :data-query="value ? '' : undefined">
    <StudioIcon name="search" />
    <input
      ref="box"
      type="search"
      spellcheck="false"
      autocomplete="off"
      :value="value"
      :aria-label="t('chat.search_sessions_or_projects')"
      :placeholder="t('chat.search_sessions_or_projects')"
      data-action-keydown="workspace.search"
      @input="onInput"
      @keydown="onKey"
    />
    <!-- The same press the palette's own shortcut makes, printed with the
         combination it really is: a label has to name what happens. -->
    <button
      v-if="!value"
      type="button"
      class="clr"
      data-action="workspace.search"
      data-value="palette"
      :aria-label="t('session.search_sessions_and_actions')"
      :title="`${t('session.search_sessions_and_actions')} · ${shortcut}`"
      @click="emit('palette')"
    ><kbd>{{ shortcut }}</kbd></button>
    <button v-else type="button" class="clr" data-action="workspace.search" data-value="clear" :aria-label="t('common.clear_search')" :title="t('common.clear_search')" @click="clear">
      <StudioIcon name="close" />
    </button>
  </div>
</template>
