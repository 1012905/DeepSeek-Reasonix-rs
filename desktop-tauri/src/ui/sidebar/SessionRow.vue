<script lang="ts">
// What one conversation row is given. Declared in a plain block so the type can
// be imported without the component (a template-only `defineProps` cannot be
// picked up by another module).
export interface SessionRowProps {
  sessionPath: string;
  label: string;
  /** The pane on screen drives this conversation. */
  active: boolean;
  unread: boolean;
  archived: boolean;
  pinned: boolean;
  isolated?: boolean;
  /** The search word, so the matching part of the title is marked. */
  needle: string;
  onOpen: (path: string) => void;
  onRename?: (path: string, title: string) => void;
  onArchive?: (path: string) => void;
  onDelete?: (path: string) => void;
  onMenu: (path: string, trigger: HTMLElement) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import Hit from "../Hit.vue";
import { LeadDot } from "../UnreadMark.vue";

const props = defineProps<SessionRowProps>();

const row = ref<HTMLElement | null>(null);
const editing = ref(false);
const draft = ref("");
const field = ref<HTMLInputElement | null>(null);

// Whether there is anything to do to this row at all, which decides whether the
// hover buttons are drawn: a button that calls nothing is a promise it cannot keep.
const acts = computed(() => !!props.onRename || !!props.onArchive || !!props.onDelete);

async function startRename() {
  if (!props.onRename) return;
  draft.value = props.label;
  editing.value = true;
  await nextTick();
  field.value?.focus();
  field.value?.select();
}

function commit() {
  if (!editing.value) return;
  const next = draft.value.trim();
  editing.value = false;
  // An unchanged title is not a write: saving what the row already says would
  // ask the kernel for a new revision of the same words.
  if (next && next !== props.label.trim()) props.onRename?.(props.sessionPath, next);
}

function cancel() {
  draft.value = props.label;
  editing.value = false;
  row.value?.focus();
}

function leave(event: KeyboardEvent) {
  (event.currentTarget as HTMLInputElement).blur();
}

function onKey(event: KeyboardEvent) {
  // While the field is open it owns its keys: the tree's arrows move a caret
  // there, and Enter there means "save", not "open".
  if (editing.value) return;
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    props.onOpen(props.sessionPath);
  } else if (event.key === "F2" && props.onRename) {
    event.preventDefault();
    void startRename();
  } else if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
    event.preventDefault();
    if (row.value) props.onMenu(props.sessionPath, row.value);
  }
}

function onContext(event: MouseEvent) {
  // A field or a menu inside the row owns the press; only the row's own space
  // raises the conversation's menu.
  if ((event.target as HTMLElement).closest("input, [role='menu']")) return;
  event.preventDefault();
  event.stopPropagation();
  if (row.value) props.onMenu(props.sessionPath, row.value);
}
</script>

<template>
  <div
    ref="row"
    class="sessrow ds-row"
    role="treeitem"
    :aria-level="2"
    :aria-label="label"
    :aria-selected="active"
    :data-target="sessionPath"
    :data-on="active ? '' : undefined"
    :data-unread="unread && !active ? '' : undefined"
    :data-busy="editing ? '' : undefined"
    tabindex="-1"
    :title="label"
    @click="!editing && onOpen(sessionPath)"
    @dblclick.stop="startRename()"
    @keydown="onKey"
    @contextmenu="onContext"
  >
    <LeadDot :unread="unread && !active" />
    <input
      v-if="editing"
      ref="field"
      v-model="draft"
      class="sessedit"
      :aria-label="t('common.rename_this_session')"
      :placeholder="t('common.chat_title')"
      spellcheck="false"
      @click.stop
      @blur="commit"
      @keydown.enter.prevent="leave"
      @keydown.esc.stop.prevent="cancel"
    />
    <span v-else class="sesstitle ds-title">
      <span><Hit v-if="needle" :text="label" :needle="needle" /><template v-else>{{ label }}</template></span>
      <em v-if="isolated" class="sessarch">{{ t("common.isolated") }}</em>
      <em v-if="archived" class="sessarch">{{ t("common.archived") }}</em>
    </span>
    <span v-if="pinned" class="ds-pin" :aria-label="t('common.pinned')"><StudioIcon name="pin" /></span>
    <span v-if="acts" class="sessquick" :aria-label="t('session.session_row_buttons')">
      <button
        v-if="onRename"
        type="button"
        data-action="session.rename-start"
        :data-target="sessionPath"
        :title="t('common.rename_this_session')"
        :aria-label="t('common.rename_this_session')"
        @click.stop="startRename()"
      ><StudioIcon name="edit" /></button>
      <button
        v-if="onArchive"
        type="button"
        data-action="session.archive"
        :data-target="sessionPath"
        :data-value="archived ? 'restore' : 'archive'"
        :title="t(archived ? 'common.unarchive' : 'common.archive_session')"
        :aria-label="t(archived ? 'common.unarchive' : 'common.archive_session')"
        @click.stop="onArchive?.(sessionPath)"
      ><StudioIcon name="archive" /></button>
      <button
        v-if="onDelete"
        type="button"
        data-action="session.delete"
        :data-target="sessionPath"
        :title="t('common.delete_session')"
        :aria-label="t('common.delete_session')"
        @click.stop="onDelete?.(sessionPath)"
      ><StudioIcon name="trash" /></button>
    </span>
    <button
      type="button"
      class="ds-more"
      data-action="session.menu"
      :data-target="sessionPath"
      :title="t('common.session_actions')"
      :aria-label="t('common.session_actions')"
      aria-haspopup="menu"
      @click.stop="row && onMenu(sessionPath, row)"
    ><StudioIcon name="more" /></button>
  </div>
</template>
