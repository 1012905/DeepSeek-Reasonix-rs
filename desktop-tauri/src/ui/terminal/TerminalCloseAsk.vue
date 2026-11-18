<script lang="ts">
export interface TerminalCloseAskProps {
  /** The control that asked for the close, so the bubble lands beside it. */
  anchor: HTMLElement;
  onConfirm: () => void;
  onCancel: () => void;
}
</script>

<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { t } from "../../i18n";
import { pinToViewport } from "../place";

const props = defineProps<TerminalCloseAskProps>();
const box = ref<HTMLDivElement | null>(null);

onMounted(async () => {
  await nextTick();
  const el = box.value;
  if (!el) return;
  const at = props.anchor.getBoundingClientRect();
  pinToViewport(el, at.left, at.bottom + 4);
  el.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
});

const key = (e: KeyboardEvent) => {
  if (e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  props.onCancel();
};
</script>

<template>
  <Teleport to="body">
    <div
      ref="box"
      class="workbench-close-ask"
      role="alertdialog"
      :aria-label="t('terminal.close_a_running_one')"
      data-action-keydown="layer.dismiss"
      @keydown="key"
    >
      <p>{{ t("terminal.close_a_running_one") }}</p>
      <div>
        <button type="button" data-action="terminal.close-confirm" @click="onConfirm">{{ t("terminal.close_and_end") }}</button>
        <button type="button" data-action="terminal.close-cancel" @click="onCancel">{{ t("common.cancel") }}</button>
      </div>
    </div>
  </Teleport>
</template>
