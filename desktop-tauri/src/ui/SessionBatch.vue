<script lang="ts">
import { computed, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import type { TreeSession } from "../port/hub";

type SetStateAction<T> = T | ((prev: T) => T);

/** The selected rows that are still on screen; a row that leaves the displayed set leaves the selection. */
export function useSelectedRows(
  displayed: MaybeRefOrGetter<TreeSession[]>,
  selection: MaybeRefOrGetter<Set<string> | null>,
  setSelection: (next: SetStateAction<Set<string> | null>) => void,
): Ref<TreeSession[]> {
  const key = computed(() => toValue(displayed).map((row) => row.path).join("\n"));
  watch(key, (k) => {
    const paths = new Set(k ? k.split("\n") : []);
    setSelection((current) => {
      if (!current) return current;
      const next = new Set([...current].filter((path) => paths.has(path)));
      return next.size === current.size ? current : next;
    });
  }, { immediate: true });
  return computed(() => [...new Map(toValue(displayed).filter((row) => toValue(selection)?.has(row.path)).map((row) => [row.path, row])).values()]);
}
</script>

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { current, t } from "../i18n";
import type { HubPort } from "../port/hub";
import { clearDraftForSession } from "./drafts";

const props = defineProps<{
  hub: HubPort;
  selection: Set<string> | null;
  selected: TreeSession[];
  displayed: TreeSession[];
  onSelect: (next: Set<string> | null) => void;
  onExit: () => void;
  onBusy: (busy: boolean) => void;
  onClose: (ids: string[]) => Promise<void>;
  onArchive: (path: string, archived: boolean, runtimeId?: string) => Promise<void>;
  liveIds: (ids: string[]) => string[];
  reload: () => Promise<void>;
  onError: (e: unknown) => void;
}>();

const phase = ref<"idle" | "confirm" | "archive" | "restore" | "delete">("idle");
const wasSelecting = ref(props.selection !== null);
const deleteButton = ref<HTMLButtonElement | null>(null);
const doneButton = ref<HTMLButtonElement | null>(null);
const confirmCancel = ref<HTMLButtonElement | null>(null);
const pending = computed(() => phase.value !== "idle" && phase.value !== "confirm");
const running = computed(() => props.liveIds(props.selected.flatMap((row) => row.runtimeId ? [row.runtimeId] : [])).length > 0);
const blocked = computed(() => pending.value || running.value || props.selected.length === 0);
const all = computed(() => props.displayed.length > 0 && props.displayed.every((row) => props.selection?.has(row.path)));
const restore = computed(() => props.selected.length > 0 && props.selected.every((row) => row.archived));
const names = computed(() => props.selected.map((row) => row.title || row.name).join(current() === "zh" ? "、" : ", "));
const cancel = () => { phase.value = "idle"; props.onBusy(false); deleteButton.value?.focus(); };

watch(() => props.selection !== null, (isSelecting) => {
  if (wasSelecting.value && !isSelecting) props.onExit();
  wasSelecting.value = isSelecting;
});

const focusEntry = () => {
  if (phase.value === "idle" && props.selection !== null) (deleteButton.value?.disabled ? doneButton.value : deleteButton.value)?.focus();
  // The confirm dialog's cancel button carries React's autoFocus.
  if (phase.value === "confirm") confirmCancel.value?.focus();
};
onMounted(focusEntry);
watch([phase, () => props.selection !== null], focusEntry, { flush: "post" });

const run = async (action: "archive" | "restore" | "delete") => {
  if (blocked.value) return;
  phase.value = action;
  props.onBusy(true);
  const failed = new Set<string>();
  let firstError: unknown;
  let removed = false;
  for (const row of props.selected) {
    try {
      if (row.runtimeId && props.liveIds([row.runtimeId]).length > 0) throw new Error(t("session.selected_sessions_are_running_stop"));
      if (action === "delete") {
        if (row.runtimeId) await props.onClose([row.runtimeId]);
        await props.hub.removeSession(row.path);
        clearDraftForSession("", row.path);
        removed = true;
      } else {
        await props.onArchive(row.path, action === "archive", row.runtimeId);
      }
    } catch (e) {
      failed.add(row.path);
      firstError ??= e;
    }
  }
  if (removed) {
    try { await props.reload(); } catch (e) { firstError ??= e; }
  }
  props.onSelect(failed);
  phase.value = "idle";
  props.onBusy(false);
  if (firstError) props.onError(firstError);
  deleteButton.value?.focus();
};

const toggleAll = () => {
  const next = new Set(props.selection);
  for (const row of props.displayed) { if (all.value) next.delete(row.path); else next.add(row.path); }
  props.onSelect(next);
};
const onConfirmKeydown = (ev: KeyboardEvent) => { if (ev.key === "Escape") { ev.stopPropagation(); cancel(); } };
</script>

<template>
  <div v-if="selection !== null" class="session-batch" :aria-busy="pending">
    <div class="session-batch-head">
      <span role="status">{{ t("session.n_sessions_selected", { n: selected.length }) }}</span>
      <button ref="doneButton" data-action="session.selection" data-value="end" :disabled="pending" @click="cancel(); onSelect(null)">{{ t("session.done_selecting") }}</button>
    </div>
    <div class="session-batch-actions">
      <button data-action="session.selection" :data-value="all ? 'clear' : 'all'" :disabled="phase !== 'idle' || displayed.length === 0"
        :aria-label="t(all ? 'session.deselect_displayed_sessions' : 'session.select_all_displayed_sessions')"
        @click="toggleAll">{{ t(all ? "session.deselect_all" : "session.select_visible") }}</button>
      <button data-action="session.batch-archive" :data-value="restore ? 'restore' : 'archive'" :disabled="blocked || phase === 'confirm'"
        :aria-label="t(restore ? 'session.unarchive_selected_sessions' : 'session.archive_selected_sessions')" @click="run(restore ? 'restore' : 'archive')">{{ t(restore ? "common.unarchive" : "common.archived_2") }}</button>
      <button ref="deleteButton" data-action="session.batch-delete" :disabled="blocked || phase === 'confirm'" :aria-label="t('session.delete_selected_sessions')"
        @click="phase = 'confirm'; onBusy(true)">{{ t("common.delete") }}</button>
    </div>
    <p v-if="running">{{ t("session.selected_sessions_are_running_stop") }}</p>
    <p v-if="pending" role="status">{{ t("session.processing_selected_sessions") }}</p>
    <div v-if="phase === 'confirm'" class="wsconfirm" role="alertdialog" :aria-label="t('session.delete_n_sessions', { n: selected.length })"
      data-action-keydown="layer.dismiss" @keydown="onConfirmKeydown">
      <div class="wsconfirm-t">
        <span class="q">{{ t("session.delete_n_sessions", { n: selected.length }) }}</span>
        <span class="session-batch-names">{{ names }}</span>
        <span class="h">{{ t("common.deletes_it_together_with_its") }}</span>
      </div>
      <div class="wsconfirm-a">
        <button ref="confirmCancel" data-action="layer.dismiss" @click="cancel">{{ t("common.cancel") }}</button>
        <button data-action="session.batch-delete" data-danger="" :disabled="blocked" @click="run('delete')">{{ t("common.delete") }}</button>
      </div>
    </div>
  </div>
</template>