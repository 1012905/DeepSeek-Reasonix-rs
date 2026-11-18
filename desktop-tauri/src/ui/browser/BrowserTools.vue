<script lang="ts">
export interface BrowserToolsProps {
  /** A page is showing, so there is something to act on. */
  page: boolean;
  /** The shell can let a person point at an element of the page. */
  canPick: boolean;
  picking: boolean;
  /** A picture is standing in for the page: the shell's own freeze is on. */
  still: boolean;
  /** A person has taken the browser over, so the way back belongs here. */
  manual: boolean;
  onPick: () => void;
  onShot: () => void;
  onExit: () => void;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";

const props = defineProps<BrowserToolsProps>();

const pickLabel = computed(() => (props.picking ? t("workbench.picking_element") : t("workbench.pick_element")));
// The shot is the shell's own freeze: the page goes away and the picture it
// showed stands in its place until the same button puts it back. It is a
// toggle, so aria-pressed says which state it is in and the name stays put.
const shotLabel = t("feedback.capture_this_window");
</script>

<template>
  <TooltipButton
    v-if="canPick"
    class="bbtn"
    data-action="browser.pick"
    :aria-pressed="picking"
    :disabled="!page"
    :aria-label="pickLabel"
    :title="pickLabel"
    @click="props.onPick()"
  >
    <StudioIcon name="target" />
  </TooltipButton>
  <TooltipButton
    class="bbtn"
    data-action="browser.shot"
    :aria-pressed="still"
    :disabled="!page"
    :aria-label="shotLabel"
    :title="shotLabel"
    @click="props.onShot()"
  >
    <StudioIcon name="image" />
  </TooltipButton>
  <TooltipButton
    v-if="manual"
    class="bbtn"
    data-action="browser.manual"
    data-value="off"
    :aria-label="t('app.close_workbench')"
    :title="t('app.close_workbench')"
    @click="props.onExit()"
  >
    <StudioIcon name="close" />
  </TooltipButton>
</template>
