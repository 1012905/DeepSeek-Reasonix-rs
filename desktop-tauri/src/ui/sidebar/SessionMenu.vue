<script lang="ts">
// The row menu, drawn at the document root so the column's clipping cannot cut
// it. Every entry reports one choice; closing is the caller's, so the same menu
// serves hover, keyboard and right-click alike.
export interface SessionMenuProps {
  label: string;
  /** session.path, carried on each entry as data-target. */
  path: string;
  archived: boolean;
  /** A pane drives this conversation. Not a mid-turn signal. */
  held: boolean;
  /** The conversation behind this row is running a turn. */
  live: boolean;
  pinned: boolean;
  exporting: boolean;
  autoNaming: boolean;
  canRename: boolean;
  onRename: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onPin?: () => void;
  onAutoName?: () => void;
  onMarkRead?: () => void;
  onCopyInfo?: () => void;
  onExport?: (format: "json" | "md") => void;
  onPause?: () => void;
  /** Named apart from the dismiss emit ("close"): Vue resolves emit("close") to
   *  the onClose listener prop, so an action named onClose would fire on every
   *  dismissal, not only this entry. */
  onCloseConversation?: () => void;
  onBranch?: () => void;
  onMove?: (direction: -1 | 1) => void;
  canMove?: { up: boolean; down: boolean };
  /** Opens the choice of another project; absent where a conversation cannot
   *  move at all. A held conversation offers it locked, not absent. */
  onMoveTo?: () => void;
  /** How many earlier versions and recovery copies this conversation kept. */
  keptCount?: number;
  keptOpen?: boolean;
  onKept?: () => void;
}
</script>

<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

defineProps<SessionMenuProps>();
const emit = defineEmits<{ (e: "close"): void }>();

const box = ref<HTMLElement | null>(null);
const rows = () => [...(box.value?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)') ?? [])];

// The choice runs before close is announced: the caller's close handler clears
// the menu target, so an entry that reads it afterwards would find nothing.
function take(done: () => void) {
  done();
  emit("close");
}

// Arrows walk the entries, because a menu that opens on its first row and cannot
// be walked with a key is a list, not a menu. Escape and Tab both leave, and the
// caller hands focus back to the row that opened this one.
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

defineExpose({
  el: () => box.value,
  focus: () => rows()[0]?.focus(),
});
</script>

<template>
  <div ref="box" class="session-pop" role="menu" :aria-label="t('common.session_actions')" @keydown="onKey" @click.stop>
    <div class="session-pop-head">
      <b>{{ label }}</b>
      <small>{{ live ? t("session.running") : t("common.done_2") }} · {{ t("session.local_workspace") }}</small>
    </div>
    <template v-if="held && live && onPause">
      <div class="session-pop-group">
        <button role="menuitem" data-action="session.pause" :data-target="path" @click="take(onPause!)">
          <StudioIcon name="pause" /><span>{{ t("session.pause_run") }}</span>
        </button>
      </div>
      <div class="session-pop-divider" />
    </template>
    <template v-if="onPin || (onMove && canMove)">
      <div class="session-pop-group">
        <button v-if="onPin" role="menuitem" data-action="session.pin" :data-target="path" @click="take(onPin!)">
          <StudioIcon name="pin" /><span>{{ pinned ? t("common.unpin") : t("common.pin_session") }}</span>
        </button>
        <template v-if="onMove && canMove">
          <button v-for="direction in [-1, 1]" :key="direction" role="menuitem" data-action="session.move" :data-target="path" :data-value="direction"
            :disabled="direction === -1 ? !canMove.up : !canMove.down" @click="take(() => onMove!(direction as -1 | 1))">
            <StudioIcon name="arrow" :data-flip="direction === 1 ? '' : undefined" /><span>{{ t(direction === -1 ? "common.move_up" : "common.move_down") }}</span>
          </button>
        </template>
      </div>
      <div class="session-pop-divider" />
    </template>
    <template v-if="canRename || onAutoName || onMarkRead">
      <div class="session-pop-group">
        <button v-if="canRename" role="menuitem" data-action="session.rename-start" :data-target="path" @click="take(onRename)">
          <StudioIcon name="edit" /><span>{{ t("common.renamed") }}</span>
        </button>
        <button v-if="onAutoName" role="menuitem" data-action="session.auto-name" :data-target="path" :disabled="autoNaming" @click="take(onAutoName!)">
          <StudioIcon name="spark" /><span>{{ autoNaming ? t("session.naming") : t("session.auto_name") }}</span>
        </button>
        <button v-if="onMarkRead" role="menuitem" data-action="session.mark-read" :data-target="path" @click="take(onMarkRead!)">
          <StudioIcon name="check" /><span>{{ t("common.mark_as_read") }}</span>
        </button>
      </div>
      <div class="session-pop-divider" />
    </template>
    <template v-if="onCopyInfo || onExport || (held && onBranch)">
      <div class="session-pop-group">
        <button v-if="held && onBranch" role="menuitem" data-action="session.branch" :data-target="path" @click="take(onBranch!)">
          <StudioIcon name="branch" /><span>{{ t("session.branch_from_the_latest_reply") }}</span>
        </button>
        <button v-if="onCopyInfo" role="menuitem" data-action="session.copy-info" :data-target="path" @click="take(onCopyInfo!)">
          <StudioIcon name="copy" /><span>{{ t("session.copy_session_information") }}</span>
        </button>
        <div v-if="onExport" class="session-pop-export" role="group" :aria-label="t('session.export_session')">
          <StudioIcon name="download" /><span>{{ t("session.export_session") }}</span>
          <button role="menuitem" data-action="session.export" data-value="md" :data-target="path" :disabled="exporting"
            :aria-label="`${t('session.export_session')} · Markdown`" @click="take(() => onExport!('md'))">Markdown</button>
          <button role="menuitem" data-action="session.export" data-value="json" :data-target="path" :disabled="exporting"
            :aria-label="`${t('session.export_session')} · JSON`" @click="take(() => onExport!('json'))">JSON</button>
        </div>
      </div>
      <div class="session-pop-divider" />
    </template>
    <div class="session-pop-group">
      <button v-if="(keptCount ?? 0) > 0 && onKept" role="menuitem" data-action="session.kept" :data-target="path" :aria-expanded="keptOpen" @click="take(onKept!)">
        <StudioIcon name="clock" /><span>{{ t("session.earlier_versions_n", { n: keptCount ?? 0 }) }}</span>
      </button>
      <button v-if="onMoveTo" role="menuitem" :aria-label="t('session.move_to_a_project')" :aria-describedby="held ? 'session-move-why' : undefined"
        data-action="session.move-to-start" :data-target="path" :disabled="held" @click="take(onMoveTo!)">
        <StudioIcon name="folder" /><span>{{ t("session.move_to_a_project") }}<small v-if="held" class="session-pop-why" id="session-move-why">{{ t("session.close_this_conversation_first_then") }}</small></span>
      </button>
      <button role="menuitem" data-action="session.archive" :data-target="path" :data-value="archived ? 'restore' : 'archive'" :disabled="held && live" @click="take(onArchive)">
        <StudioIcon name="archive" /><span>{{ t(archived ? "common.unarchive" : "common.archive_session") }}</span>
      </button>
      <button v-if="held && onCloseConversation" role="menuitem" data-action="session.close" :data-target="path" :disabled="live"
        :aria-label="t('session.close_conversation')" :aria-describedby="live ? 'session-close-why' : undefined" @click="take(onCloseConversation!)">
        <StudioIcon name="close" /><span>{{ t("session.close_conversation") }}<small v-if="live" class="session-pop-why" id="session-close-why">{{ t("session.running_pause_it_before_closing") }}</small></span>
      </button>
      <button class="danger" role="menuitem" data-action="session.delete" :data-target="path" @click="take(onDelete)">
        <StudioIcon name="trash" /><span>{{ t("common.delete_session") }}</span>
      </button>
    </div>
  </div>
</template>
