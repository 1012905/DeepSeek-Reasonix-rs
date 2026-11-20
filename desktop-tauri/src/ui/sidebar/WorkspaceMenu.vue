<script lang="ts">
// The workspace header's menu. It carries the one action this column can take
// on a project — taking it off the list — and says that no file moves, because
// a menu row named "remove" beside a folder is read as "delete" everywhere else.
export interface WorkspaceMenuProps {
  name: string;
  root: string;
  onArchive: () => void;
}
</script>

<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

defineProps<WorkspaceMenuProps>();
const emit = defineEmits<{ (e: "close"): void }>();

const box = ref<HTMLElement | null>(null);
const rows = () => [...(box.value?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)') ?? [])];

function take(done: () => void) {
  emit("close");
  done();
}

// Same walking as the row menu: arrows inside, Escape and Tab out, and the
// caller puts focus back on the header that opened this.
function onKey(event: KeyboardEvent) {
  if (event.key === "Escape" || event.key === "Tab") {
    event.stopPropagation();
    if (event.key === "Escape") event.preventDefault();
    emit("close");
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  event.stopPropagation();
  const all = rows();
  if (all.length === 0) return;
  const at = all.indexOf(document.activeElement as HTMLButtonElement);
  const to = event.key === "Home" ? 0
    : event.key === "End" ? all.length - 1
    : (at + (event.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
  all[to]?.focus();
}

defineExpose({ el: () => box.value, focus: () => rows()[0]?.focus() });
</script>

<template>
  <div ref="box" class="session-pop" role="menu" :aria-label="t('common.more_actions')" @keydown="onKey" @click.stop>
    <div class="session-pop-head">
      <b>{{ name }}</b>
      <small class="session-pop-path" :title="root">{{ root }}</small>
    </div>
    <div class="session-pop-group">
      <button role="menuitem" data-action="workspace.remove" @click="take(onArchive)">
        <StudioIcon name="close" /><span>{{ t("common.remove_from_the_list") }}</span><small>{{ t("common.keeps_files") }}</small>
      </button>
    </div>
  </div>
</template>
