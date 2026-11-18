<script lang="ts">
export interface FileMenu {
  path: string;
  folder: boolean;
  x: number;
  y: number;
  /** The row the menu belongs to: focus goes back to it when the menu closes. */
  anchor: HTMLElement;
}

export interface WorkbenchFileMenuProps {
  menu: FileMenu;
  root: string;
  revealable: boolean;
  platform: string;
  onReveal: (path: string) => void;
  onAddToChat?: (reference: string) => void;
  onOpen?: (path: string) => void;
  onClose: () => void;
}

/** The path as this machine spells it, or "" when root is not an absolute path
 *  (a remote workspace), where a copyable absolute path would be a lie. */
export function absoluteWorkspacePath(root: string, path: string): string {
  if (!root.startsWith("/") && !/^[a-z]:[\\/]/i.test(root) && !/^\\\\[^\\]+\\[^\\]+/.test(root)) return "";
  const separator = root.startsWith("\\\\") || /^[a-z]:\\/i.test(root) ? "\\" : "/";
  const base = root.replace(separator === "\\" ? /[\\/]+$/ : /\/+$/, "");
  return base + separator + (separator === "\\" ? path.replaceAll("/", "\\") : path);
}
</script>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { t } from "../../i18n";
import { pushNote } from "../../state/toasts";
import { copyText } from "../CopyButton.vue";
import { fileReference } from "../fileReference";
import { pinToViewport } from "../place";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<WorkbenchFileMenuProps>();

const box = ref<HTMLDivElement | null>(null);
const state = ref<"idle" | "copying" | "failed">("idle");
let live = true;

const shut = () => { props.onClose(); };
const back = () => {
  props.menu.anchor.focus({ preventScroll: true });
  shut();
};

const copy = async (text: string, said: string) => {
  state.value = "copying";
  try {
    await copyText(text);
    pushNote(t("workbench.copied_text", { text: said }));
    if (live) back();
  } catch {
    if (live) state.value = "failed";
  }
};

// The menu closes on a press anywhere it does not hold, on Escape, on Tab, and
// when the window loses focus — a menu left over a window that is no longer in
// front belongs to nothing.
const away = (event: Event) => {
  if (box.value?.contains(event.target as Node)) return;
  shut();
};
const key = (e: KeyboardEvent) => {
  if (e.key !== "Escape" && e.key !== "Tab") return;
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
  }
  back();
};

onMounted(() => {
  if (box.value) pinToViewport(box.value, props.menu.x, props.menu.y);
  box.value?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
  addEventListener("click", away);
  addEventListener("keydown", key, true);
  addEventListener("blur", shut);
});

onBeforeUnmount(() => {
  live = false;
  removeEventListener("click", away);
  removeEventListener("keydown", key, true);
  removeEventListener("blur", shut);
});

const nav = (e: KeyboardEvent) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const items = [...(box.value?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
  const at = items.indexOf(document.activeElement as HTMLButtonElement);
  items[(at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
};

const reference = () => fileReference(props.menu.path + (props.menu.folder ? "/" : ""));
const ambiguous = () => props.menu.path.includes(":");
</script>

<template>
  <Teleport to="body">
    <div
      ref="box"
      class="tabmenu"
      data-action-keydown="layer.dismiss"
      role="menu"
      :aria-label="t('common.more')"
      @keydown="nav"
      @click="(e) => e.stopPropagation()"
    >
      <div class="tabmenu-head">
        <b>{{ menu.path.split("/").at(-1) + (menu.folder ? "/" : "") }}</b>
        <span>{{ menu.path.includes("/") ? menu.path.slice(0, menu.path.lastIndexOf("/")) : t("workbench.root_folder") }}</span>
      </div>
      <button
        v-if="onAddToChat"
        role="menuitem"
        :disabled="!reference()"
        data-action="workbench.add-to-chat"
        :data-target="menu.path"
        :title="reference() ? undefined : ambiguous() ? t('workbench.this_name_could_resolve_to') : t('common.this_name_cannot_be_represented')"
        @click="onClose(); onAddToChat(reference())"
      >
        <StudioIcon name="chat" /><span>{{ t("workbench.add_to_conversation") }}</span>
      </button>
      <button
        role="menuitem"
        :disabled="!reference() || state === 'copying'"
        data-action="workbench.copy-ref"
        :data-target="menu.path"
        :aria-label="t('workbench.copy_reference')"
        :title="reference() ? undefined : ambiguous() ? t('workbench.this_name_could_resolve_to') : t('common.this_name_cannot_be_represented')"
        @click="void copy(reference(), reference())"
      >
        <StudioIcon name="quote" />
        <span>
          {{ t("workbench.copy_reference") }}
          <small v-if="!reference()">{{ ambiguous() ? t("workbench.this_name_could_resolve_to") : t("common.this_name_cannot_be_represented") }}</small>
        </span>
        <kbd v-if="reference()">{{ reference() }}</kbd>
      </button>
      <button
        role="menuitem"
        :disabled="!absoluteWorkspacePath(root, menu.path) || state === 'copying'"
        data-action="workbench.copy-path"
        :data-target="menu.path"
        @click="void copy(absoluteWorkspacePath(root, menu.path), menu.path)"
      >
        <StudioIcon name="copy" /><span>{{ t("common.copy_path") }}</span>
      </button>
      <hr v-if="revealable || (onOpen && !menu.folder)" />
      <button v-if="revealable" role="menuitem" data-action="workbench.reveal" :data-target="menu.path" @click="onClose(); onReveal(menu.path)">
        <StudioIcon name="reveal" />
        <span>{{ platform === "darwin" ? t("workbench.reveal_in_finder") : platform === "windows" ? t("workbench.reveal_in_file_explorer") : t("workbench.show_in_file_manager") }}</span>
      </button>
      <button v-if="onOpen && !menu.folder" role="menuitem" data-action="workbench.open-file" :data-target="menu.path" @click="onClose(); onOpen(menu.path)">
        <StudioIcon name="external" /><span>{{ t("workbench.open_with_default_application") }}</span>
      </button>
      <div v-if="state === 'failed'" role="alert">{{ t("common.copy_failed_try_again") }}</div>
    </div>
  </Teleport>
</template>
