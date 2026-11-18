<script lang="ts">
// One project and the conversations drawn under it. The fold itself, the rows
// it shows and the search word all arrive settled from the caller: this
// component decides only what a project header looks like and which of its own
// controls are offered.
import type { TreeSession } from "../../port/hub";

export interface WorkspaceGroupProps {
  root: string;
  name: string;
  /** The folder this project is filed under, which tells two projects of the
   *  same name apart. Empty when there is nothing above it. */
  parent: string;
  /** Whatever the kernel reports about the branch; absent draws no chip. */
  branch?: string;
  /** The conversation writes to a copy rather than to the project's own folder. */
  isolated?: boolean;
  /** Extra folders this project may also write to, as the kernel listed them. */
  extra: number;
  /** Whether there is anything inside. A project with no rows is not foldable. */
  foldable: boolean;
  shut: boolean;
  /** A project the window says has more conversations than the tree carried. */
  more: number;
  unread: number;
  /** Rows already narrowed and ordered by the caller. */
  rows: TreeSession[];
  needle: string;
  activePath?: string;
  busy: boolean;
  locked: boolean;
  onFold: (root: string, shut: boolean) => void;
  onNewSession: (root: string) => void;
  onMenu: (root: string, trigger: HTMLElement) => void;
  onOpen: (path: string) => void;
  onRename?: (path: string, title: string) => void;
  onArchive?: (path: string) => void;
  onDelete?: (path: string) => void;
  onSessionMenu: (path: string, trigger: HTMLElement) => void;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { UnreadCount } from "../UnreadMark.vue";
import SessionRow from "./SessionRow.vue";
import { rowLabel } from "./workspaceTree";

const props = defineProps<WorkspaceGroupProps>();

const head = ref<HTMLElement | null>(null);

const label = computed(() => props.name || props.root);
const unreadRows = computed(() => props.rows.filter((row) => row.unread).length);

function toggle() {
  if (props.foldable) props.onFold(props.root, !props.shut);
}

function onKey(event: KeyboardEvent) {
  if (event.target !== event.currentTarget) return;
  if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
  event.preventDefault();
  event.stopPropagation();
  if (head.value) props.onMenu(props.root, head.value);
}

function onContext(event: MouseEvent) {
  if ((event.target as HTMLElement).closest("input, button, [role='menu']")) return;
  event.preventDefault();
  event.stopPropagation();
  if (head.value) props.onMenu(props.root, head.value);
}
</script>

<template>
  <div class="wsnode" :data-current="rows.some((row) => row.path === activePath) ? '' : undefined" :data-missing="locked ? '' : undefined">
    <div
      ref="head"
      class="wsrow ds-proj"
      role="treeitem"
      :aria-level="1"
      aria-haspopup="menu"
      :aria-expanded="foldable ? !shut : undefined"
      :aria-label="label"
      :data-target="root"
      tabindex="-1"
      @click="toggle()"
      @keydown="onKey"
      @contextmenu="onContext"
    >
      <StudioIcon
        name="down"
        class="wschev ds-chev"
        :data-open="foldable && !shut ? '' : undefined"
        :data-leaf="foldable ? undefined : ''"
      />
      <StudioIcon name="folder" class="wsfolder ds-folder" />
      <span class="wsname ds-title">{{ label }}</span>
      <!-- A tag is this project's identity, so it takes the second line and the
           name keeps the row. -->
      <span v-if="isolated || parent || extra > 0" class="wsmeta">
        <em v-if="isolated" class="wstag">{{ t("common.isolated") }}</em>
        <em v-else-if="parent" class="wstag">{{ parent }}</em>
        <em v-if="extra > 0" class="wstag" :title="t('session.extra_folders_for_this_project')">{{ t("common.n", { n: extra }) }}</em>
      </span>
      <span v-if="branch" class="ds-tree" :title="branch"><StudioIcon name="branch" />{{ branch }}</span>
      <span class="wsacts">
        <UnreadCount v-if="shut" :n="unreadRows" />
        <span class="rowswap"><span class="rowbtns">
          <button
            type="button"
            class="wsadd"
            data-action="session.new"
            :data-target="root"
            :disabled="busy || locked"
            :title="t('common.start_a_new_session_in', { name: label })"
            :aria-label="t('common.start_a_new_session_in', { name: label })"
            @click.stop="onNewSession(root)"
          ><StudioIcon name="plus" /></button>
          <button
            type="button"
            class="wsmore"
            data-action="workspace.menu"
            :data-target="root"
            :title="t('common.more_actions')"
            :aria-label="label"
            aria-haspopup="menu"
            @click.stop="head && onMenu(root, head)"
          ><StudioIcon name="more" /></button>
        </span></span>
      </span>
    </div>
    <div v-if="!shut" class="kids">
      <SessionRow
        v-for="row in rows"
        :key="row.path"
        :session-path="row.path"
        :label="rowLabel(row)"
        :active="row.path === activePath"
        :unread="!!row.unread"
        :archived="!!row.archived"
        :pinned="!!row.pinned"
        :isolated="row.isolated"
        :needle="needle"
        :on-open="onOpen"
        :on-rename="onRename"
        :on-archive="onArchive"
        :on-delete="onDelete"
        :on-menu="onSessionMenu"
      />
      <!-- Not a button: this window has no page left to ask for, so the line says
           what would find the rest rather than offering a press that does nothing. -->
      <p v-if="more > 0" class="sessnote">{{ t("session.n_older_conversations_use_search", { n: more }) }}</p>
    </div>
  </div>
</template>
