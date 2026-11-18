<script lang="ts">
export interface BrowserFullToggleProps {
  full: boolean;
  onToggle: () => void;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import { ariaChord, chord } from "../keys";
import StudioIcon from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";

const props = defineProps<BrowserFullToggleProps>();

const label = computed(() => (props.full ? t("workbench.exit_expanded_workbench") : t("workbench.expand_workbench")));
// Shift+F reads as a modifier chord here (see browserfull.ts), not Cmd+Shift+F:
// aria-keyshortcuts names modifiers in words, so it is the same two keys.
const shortcut = chord("Shift+F");
const ariaKeyshortcuts = `${ariaChord("F")}+Shift`;
</script>

<template>
  <TooltipButton
    class="workbench-files"
    data-action="browser.full"
    :aria-pressed="full"
    :aria-label="label"
    :aria-keyshortcuts="ariaKeyshortcuts"
    :title="label"
    :shortcut="shortcut"
    @click="props.onToggle()"
  >
    <StudioIcon name="expand" />
  </TooltipButton>
</template>
