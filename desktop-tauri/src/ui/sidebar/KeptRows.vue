<script lang="ts">
import type { TreeSession } from "../../port/hub";

/** The conversations one conversation kept: what it read before each edit,
 *  regenerate or rewind cut it, and the recovery copies a conflicting save left
 *  behind. Each one opens as a whole conversation of its own. */
export interface KeptRowsProps {
  /** The conversation as it stood before each cut, newest first. */
  versions?: TreeSession[];
  /** Conflict-recovery copies of the same conversation. */
  copies?: TreeSession[];
  /** The conversation the window is opening right now. */
  opening?: string;
  onOpen: (path: string) => void;
  onDelete: (path: string) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import { asksDelete } from "../keys";
import StudioIcon from "../StudioIcon.vue";
import { rowLabel } from "./workspaceTree";

const props = defineProps<KeptRowsProps>();

// The one row the delete press is asking about. A kept conversation is only
// reachable from here, so a delete that ran on one press would leave nothing
// to undo it with.
const asking = ref("");
let askBox: HTMLElement | null = null;

function holdAsk(el: unknown) {
  askBox = (el as HTMLElement | null) ?? null;
}

// The question replaces its row, so a keyboard that pressed Delete would be
// left on the page behind it. The first button is the way out, not the way
// through, which is the one to land on.
watch(asking, async (path) => {
  if (!path) return;
  await nextTick();
  askBox?.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
});

const kept = computed(() => [
  ...(props.versions ?? []).map((session) => ({ session, version: true })),
  ...(props.copies ?? []).map((session) => ({ session, version: false })),
]);

function title(entry: { session: TreeSession; version: boolean }): string {
  return entry.version ? t("session.earlier_version_n_turns", { n: entry.session.turns ?? 0 }) : t("session.recovery");
}

function onKey(event: KeyboardEvent, path: string) {
  if (event.target !== event.currentTarget) return;
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    props.onOpen(path);
  } else if (asksDelete(event)) {
    event.preventDefault();
    asking.value = path;
  }
}

function drop(path: string) {
  asking.value = "";
  props.onDelete(path);
}
</script>

<template>
  <template v-for="entry in kept" :key="entry.session.path">
    <div
      v-if="asking === entry.session.path"
      :ref="holdAsk"
      class="wsconfirm"
      role="alertdialog"
      :aria-label="entry.version ? t('session.delete_this_earlier_version') : t('session.delete_this_recovery_copy')"
      :data-target="entry.session.path"
      @keydown.esc.stop.prevent="asking = ''"
    >
      <div class="wsconfirm-t">
        <span class="q">{{ entry.version ? t("session.delete_this_earlier_version") : t("session.delete_this_recovery_copy") }}</span>
        <span class="h">{{ t("common.deletes_it_together_with_its") }}</span>
      </div>
      <div class="wsconfirm-a">
        <button type="button" data-action="layer.dismiss" @click="asking = ''">{{ t("common.cancel") }}</button>
        <button type="button" data-action="session.delete" :data-target="entry.session.path" data-danger="" @click="drop(entry.session.path)">{{ t("common.delete") }}</button>
      </div>
    </div>
    <div
      v-else
      class="sessrow sesscopy ds-row"
      role="treeitem"
      :aria-level="3"
      :aria-selected="opening === entry.session.path"
      :data-target="entry.session.path"
      :data-opening="opening === entry.session.path ? '' : undefined"
      :data-on="opening === entry.session.path ? '' : undefined"
      :title="rowLabel(entry.session)"
      tabindex="-1"
      @click="onOpen(entry.session.path)"
      @keydown="onKey($event, entry.session.path)"
    >
      <span class="ds-lead" />
      <span class="sesstitle ds-title"><span>{{ title(entry) }}</span></span>
      <button
        type="button"
        class="wsdel"
        data-action="session.delete"
        :data-target="entry.session.path"
        :title="t('session.delete_this_session')"
        :aria-label="t('session.delete_this_session')"
        @click.stop="asking = entry.session.path"
      ><StudioIcon name="trash" /></button>
    </div>
  </template>
</template>
