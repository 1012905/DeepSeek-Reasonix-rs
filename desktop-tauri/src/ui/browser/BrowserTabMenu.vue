<script lang="ts">
export interface BrowserTabMenuState {
  /** The tab the menu was opened for, as `keyOf` spells it (`browser:<target>`). */
  target: string;
  /** What the tab is called, drawn in the menu's heading. */
  label: string;
  x: number;
  y: number;
  /** Given focus back when the menu closes; a mouse-open bubble must not leave
   *  the keyboard nowhere. */
  anchor: HTMLElement;
}

export interface BrowserTabMenuProps {
  menu: BrowserTabMenuState;
  /** How many tabs each choice closes. Said beside it, so the cost is visible. */
  others: number;
  right: number;
  all: number;
  onDismiss: () => void;
  onClose: (mode: "one" | "others" | "right" | "all") => void;
}

export type BrowserCloseMode = "one" | "others" | "right" | "all";
</script>

<script setup lang="ts">
import { onScopeDispose, ref, watch } from "vue";
import { t } from "../../i18n";
import { listenAction } from "../listen";
import { pinToViewport } from "../place";

const props = defineProps<BrowserTabMenuProps>();

const box = ref<HTMLDivElement | null>(null);

// A menu is a layer: a press anywhere else closes it, and so does the window
// losing focus — an open menu over a page that is no longer in front lies.
watch(() => props.menu, (menu, _was, onCleanup) => {
  const dismiss = (event: Event) => {
    if (!box.value?.contains(event.target as Node)) props.onDismiss();
  };
  const blur = () => props.onDismiss();
  const off = listenAction(document, "pointerdown", { action: "layer.dismiss", listener: dismiss });
  window.addEventListener("blur", blur);
  void Promise.resolve().then(() => {
    const el = box.value;
    if (!el) return;
    pinToViewport(el, menu.x, menu.y);
    el.querySelector<HTMLButtonElement>("button")?.focus();
  });
  onCleanup(() => {
    off();
    window.removeEventListener("blur", blur);
  });
}, { immediate: true, flush: "post" });

// The opener gets focus back on the way out, whether the menu closed itself or
// the whole panel went with it.
onScopeDispose(() => {
  if (props.menu.anchor.isConnected) props.menu.anchor.focus();
});

const items = (root: HTMLElement) => [...root.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];

const onKeydown = (event: KeyboardEvent) => {
  if (event.key === "Escape" || event.key === "Tab") {
    event.preventDefault();
    event.stopPropagation();
    props.onDismiss();
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const all = items(event.currentTarget as HTMLElement);
  const at = all.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? all.length - 1 : (at + (event.key === "ArrowUp" ? -1 : 1) + all.length) % all.length;
  all[next]?.focus();
};
</script>

<template>
  <Teleport to="body">
    <div ref="box" class="tabmenu" data-action-keydown="layer.dismiss" role="menu" :aria-label="t('workbench.browser_tabs')" @keydown="onKeydown">
      <div class="tabmenu-head">
        <span>{{ t("workbench.browser_tab_named", { name: menu.label }) }}</span>
      </div>
      <button role="menuitem" type="button" data-action="workbench.close" :data-target="menu.target" @click="props.onClose('one')">
        <span>{{ t("common.close") }}</span>
      </button>
      <button role="menuitem" type="button" data-action="workbench.browser-close" :data-target="menu.target" data-value="others"
        :aria-label="t('workbench.close_other_browser_tabs')" :disabled="!others" @click="props.onClose('others')">
        <span>{{ t("workbench.close_other_browser_tabs") }}</span><kbd v-if="others > 0">{{ others }}</kbd>
      </button>
      <button role="menuitem" type="button" data-action="workbench.browser-close" :data-target="menu.target" data-value="right"
        :aria-label="t('workbench.close_browser_tabs_to_the')" :disabled="!right" @click="props.onClose('right')">
        <span>{{ t("workbench.close_browser_tabs_to_the") }}</span><kbd v-if="right > 0">{{ right }}</kbd>
      </button>
      <button role="menuitem" type="button" data-action="workbench.browser-close" :data-target="menu.target" data-value="all"
        :aria-label="t('workbench.close_all_browser_tabs')" @click="props.onClose('all')">
        <span>{{ t("workbench.close_all_browser_tabs") }}</span><kbd v-if="all > 0">{{ all }}</kbd>
      </button>
    </div>
  </Teleport>
</template>
