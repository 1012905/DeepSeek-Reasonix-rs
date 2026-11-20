<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../i18n";
import { chord } from "./keys";
import { pinToViewport } from "./place";
import type { TextContextMenuRequest } from "../port/host";
import type { TextMenuAction } from "./workbench_text_menu";

const props = defineProps<{
  menu: TextContextMenuRequest;
  onAction: (action: TextMenuAction | "quote") => Promise<void>;
  onClose: (restore: boolean) => void;
}>();

const box = ref<HTMLDivElement | null>(null);

watch([() => props.menu, () => props.onClose], (_v, _o, onCleanup) => {
  const el = box.value;
  if (el) pinToViewport(el, props.menu.x, props.menu.y);
  const shut = (event: Event) => { if (!box.value?.contains(event.target as Node)) props.onClose(false); };
  const key = (e: KeyboardEvent) => {
    if (e.key !== "Escape" && e.key !== "Tab") return;
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); }
    props.onClose(true);
  };
  el?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  addEventListener("click", shut);
  addEventListener("keydown", key, true);
  addEventListener("blur", shut);
  onCleanup(() => {
    removeEventListener("click", shut);
    removeEventListener("keydown", key, true);
    removeEventListener("blur", shut);
  });
}, { immediate: true, flush: "post" });

const nav = (e: KeyboardEvent) => {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
  e.preventDefault();
  const buttons = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
  const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
  buttons[e.key === "Home" ? 0 : e.key === "End" ? buttons.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
};
</script>

<template>
  <Teleport to="body">
    <div
      ref="box"
      class="tabmenu"
      role="menu"
      :aria-label="t('workbench.text_actions')"
      data-action-keydown="layer.dismiss"
      @keydown="nav"
    >
      <template v-if="menu.quote && menu.selected">
        <button role="menuitem" data-action="workbench.quote-selection" @click="onAction('quote')">{{ t("workbench.add_to_conversation") }}</button>
        <hr />
      </template>
      <button v-if="menu.editable" role="menuitem" :disabled="!menu.selected" data-action="workbench.text-cut" @click="onAction('cut')">{{ t("workbench.cut") }}<kbd aria-hidden="true">{{ chord("X") }}</kbd></button>
      <button role="menuitem" :disabled="!menu.selected" data-action="workbench.text-copy" @click="onAction('copy')">{{ t("common.copy") }}<kbd aria-hidden="true">{{ chord("C") }}</kbd></button>
      <button v-if="menu.editable" role="menuitem" data-action="workbench.text-paste" @click="onAction('paste')">{{ t("workbench.paste") }}<kbd aria-hidden="true">{{ chord("V") }}</kbd></button>
      <button role="menuitem" data-action="workbench.text-select-all" @click="onAction('selectAll')">{{ t("workbench.select_all") }}<kbd aria-hidden="true">{{ chord("A") }}</kbd></button>
      <template v-if="menu.editable">
        <hr />
        <button role="menuitem" :disabled="!menu.canUndo" data-action="workbench.text-undo" @click="onAction('undo')">{{ t("workbench.undo") }}<kbd aria-hidden="true">{{ chord("Z") }}</kbd></button>
        <button role="menuitem" :disabled="!menu.canRedo" data-action="workbench.text-redo" @click="onAction('redo')">{{ t("workbench.redo") }}<kbd aria-hidden="true">{{ chord("⇧Z") }}</kbd></button>
      </template>
    </div>
  </Teleport>
</template>