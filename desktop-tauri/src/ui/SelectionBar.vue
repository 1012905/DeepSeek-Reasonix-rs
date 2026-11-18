<script lang="ts">
import { t } from "../i18n";
import { pushNote } from "../state/toasts";
import { copyText } from "./CopyButton.vue";

// floor is the top edge of whatever scrolls the selection, which the bar must not cross.
export interface SelectionAt { x: number; top: number; bottom: number; floor: number; text: string }

export function copySelection(text: string) {
  void copyText(text).then(() => pushNote(t("common.copied")), () => pushNote(t("common.copy_failed_try_again")));
}
</script>

<script setup lang="ts">
import { computed } from "vue";

// Room the bar needs above a selection; short of it the bar drops below.
const ROOM = 40;

/** The small bar that floats under a selection, so a passage reaches the
 *  conversation or the clipboard without the context menu. Its buttons must
 *  prevent mousedown: focus moving away would clear the selection they act on. */
const props = defineProps<{ at: SelectionAt; note?: string }>();
const below = computed(() => props.at.top - props.at.floor < ROOM);
</script>

<template>
  <Teleport to="body">
    <div class="sel-tools" role="toolbar" :aria-label="t('chat.selected_text')" :data-below="below ? '' : undefined"
      :style="below ? { left: `${at.x}px`, top: `${at.bottom + 6}px` } : { left: `${at.x}px`, top: `${at.top - 6}px` }">
      <slot />
      <small v-if="note">{{ note }}</small>
    </div>
  </Teleport>
</template>