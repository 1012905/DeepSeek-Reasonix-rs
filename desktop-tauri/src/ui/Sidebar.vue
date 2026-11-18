<script lang="ts">
import type { SidebarProps } from "./sidebar/workspaceTree";

// Re-exported so the window wires this component against one declaration. The
// interface itself lives with the tree logic, which is the only module that
// needs to name it from a place that imports no component.
export type { SidebarProps };
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { t } from "../i18n";
import { useSidebarFolds } from "../state/sidebarFolds";
import type { SessionScope } from "./sessionScope";
import StudioIcon from "./StudioIcon.vue";
import { useTreeKeys } from "./tree";
import { pinToViewport } from "./place";
import SidebarSearch, { shortcutLabel } from "./sidebar/SidebarSearch.vue";
import SessionFilter from "./sidebar/SessionFilter.vue";
import SessionMenu from "./sidebar/SessionMenu.vue";
import WorkspaceMenu from "./sidebar/WorkspaceMenu.vue";
import WorkspaceGroup from "./sidebar/WorkspaceGroup.vue";
import RecentSessions from "./sidebar/RecentSessions.vue";
import ContentMatches from "./sidebar/ContentMatches.vue";
import Palette, { type Command } from "./sidebar/Palette.vue";
import {
  SCOPES,
  SCOPE_KEY,
  allFolded,
  filterTree,
  foldableRoots,
  foldedWith,
  isShut,
  parentName,
  recentRows,
  rowLabel,
  scopeCounts,
  workspaceOf,
} from "./sidebar/workspaceTree";
import { useSessionMove } from "./useSessionMove";
import { useSessionMoveTo } from "./useSessionMoveTo";
import MoveSessionDialog from "./MoveSessionDialog.vue";
import { saveSessionFile } from "./sessionExport";
import { sessionInfoText } from "./sessionInfo";
import { copyText } from "./CopyButton.vue";

const props = defineProps<SidebarProps>();

const [folded, setFolded] = useSidebarFolds();
const treeKeys = useTreeKeys();

const query = ref("");
const scope = ref<SessionScope>("all");
const paletteOpen = ref(false);
const recentOpen = ref(false);

const searching = computed(() => query.value.trim().length > 0);
const visible = computed(() => filterTree(props.tree, query.value, scope.value));
const counts = computed(() => scopeCounts(props.tree));
const foldable = computed(() => foldableRoots(props.tree));
const allShut = computed(() => allFolded(folded.value, foldable.value));
// Five is what fits under the tree without pushing the column's own footer off
// the bottom; the whole list is one press away in the sheet.
const previewRows = computed(() => recentRows(props.tree));
const empty = computed(() => props.tree.length === 0);

const commands = computed<Command[]>(() => [
  { id: "new", label: t("common.start_a_new_session"), icon: "plus", keywords: "new session chat 新建 会话", run: () => props.onNewSession?.() },
  { id: "add", label: t("session.add_a_workspace"), icon: "folder", keywords: "add project folder workspace 添加 项目 工作区", run: () => props.onAddWorkspace() },
  { id: "recent", label: t("session.recent_sessions"), icon: "clock", keywords: "recent history 最近 历史", run: () => { recentOpen.value = true; } },
  { id: "fold", label: t(allShut.value ? "session.expand_all_projects" : "session.collapse_all_projects"), icon: "layers", keywords: "fold collapse expand projects 折叠 展开 项目", run: () => toggleAll() },
  ...(props.onOpenRemotes
    ? [{ id: "remotes", label: t("settings.other_machines"), icon: "server" as const, keywords: "remote machines ssh machines panel 远程 机器 主机", run: () => props.onOpenRemotes?.() }]
    : []),
  ...(props.onOpenSettings
    ? [{ id: "settings", label: t("common.settings"), icon: "settings" as const, keywords: "settings preferences 设置 偏好", run: () => props.onOpenSettings?.() }]
    : []),
]);

function fold(root: string, shut: boolean) {
  setFolded(foldedWith(folded.value, [root], shut));
}

function toggleAll() {
  if (foldable.value.length === 0) return;
  setFolded(foldedWith(folded.value, foldable.value, !allShut.value));
}

function openSession(path: string) {
  // The window owns what an open means — focus a held pane, or open another —
  // and it owns reporting a refusal. A throw here becomes that report rather
  // than an unhandled rejection.
  try {
    props.onOpenSession(path);
  } catch (error) {
    props.onError(error);
    return;
  }
  // Finding a conversation by name and landing on it is one gesture: the word
  // that found it is not a filter anyone should have to clear by hand.
  query.value = "";
  const workspace = workspaceOf(props.tree, path);
  if (workspace && folded.value.has(workspace.root)) fold(workspace.root, false);
}

function newSession(root: string) {
  if (folded.value.has(root)) fold(root, false);
  props.onNewSession?.(root);
}

/* ── Menus ────────────────────────────────────────────────────────────────── */

// One menu is open at a time, and the element that opened it is remembered:
// closing has to give the ring back to that row, or a keyboard is left nowhere.
// The target is a path rather than the row object, so a tree that reloads while
// the menu is open renames the conversation that is still there.
const menuAt = ref<{ of: "session"; path: string } | { of: "workspace"; root: string } | null>(null);
const sessionMenu = ref<InstanceType<typeof SessionMenu> | null>(null);
const workspaceMenu = ref<InstanceType<typeof WorkspaceMenu> | null>(null);
let opener: HTMLElement | null = null;

const menuSession = computed(() => {
  const at = menuAt.value;
  if (at?.of !== "session") return null;
  return workspaceOf(props.tree, at.path)?.sessions.find((row) => row.path === at.path) ?? null;
});
// A row is held by whichever pane drives it, focused or not; live is that pane
// running a turn. The menu gates pause/close/archive on these, not on focus.
const menuHeld = computed(() => !!menuSession.value?.runtimeId);
const menuLive = computed(() => {
  const id = menuSession.value?.runtimeId;
  return !!id && !!props.runs?.[id]?.live;
});
const menuWorkspace = computed(() => {
  const at = menuAt.value;
  if (at?.of !== "workspace") return null;
  return props.tree.find((workspace) => workspace.root === at.root) ?? null;
});

function menuElement(): HTMLElement | null {
  return sessionMenu.value?.el() ?? workspaceMenu.value?.el() ?? null;
}

function placeMenu() {
  const anchor = opener?.getBoundingClientRect();
  const el = menuElement();
  if (!anchor || !el) return;
  pinToViewport(el, anchor.right + 8, anchor.top - 7);
}

function openSessionMenu(path: string, trigger: HTMLElement) {
  if (!workspaceOf(props.tree, path)) return;
  opener = trigger;
  menuAt.value = { of: "session", path };
  void nextTick(() => {
    placeMenu();
    sessionMenu.value?.focus();
  });
}

function openWorkspaceMenu(root: string, trigger: HTMLElement) {
  if (!props.tree.some((workspace) => workspace.root === root)) return;
  opener = trigger;
  menuAt.value = { of: "workspace", root };
  void nextTick(() => {
    placeMenu();
    workspaceMenu.value?.focus();
  });
}

function closeMenu(refocus = true) {
  menuAt.value = null;
  if (refocus && opener?.isConnected) opener.focus({ preventScroll: true });
  opener = null;
}

/** An entry's action runs after the menu is gone, on the row it was opened for:
 *  the refs are read first, because closing is what empties them. */
function take(act: () => void) {
  const target = opener;
  act();
  menuAt.value = null;
  opener = null;
  if (target?.isConnected) target.focus({ preventScroll: true });
}

function renameTarget() {
  const at = menuAt.value;
  const session = menuSession.value;
  if (at?.of === "session" && session) props.onRenameSession?.(at.path, rowLabel(session));
}

function archiveTarget() {
  const at = menuAt.value;
  if (at?.of === "session") props.onArchiveSession?.(at.path);
}

function deleteTarget() {
  const at = menuAt.value;
  if (at?.of === "session") props.onDeleteSession?.(at.path);
}

function pauseTarget() {
  const id = menuSession.value?.runtimeId;
  if (id) props.onPauseSession?.(id);
}

function closeTarget() {
  const id = menuSession.value?.runtimeId;
  if (id) props.onCloseSession?.(id);
}

// The branch is named for the row it cuts from, the way the reply action names
// its own; the kernel owns the file, so the window only opens what comes back.
function branchTarget() {
  const session = menuSession.value;
  const id = session?.runtimeId;
  if (session && id) void props.onBranchSession?.(id, `${rowLabel(session)} · ${t("common.branch")}`).catch(props.onError);
}

// The column runs these itself, the way the window runs rename/archive/delete:
// the kernel owns the list, so each asks for the tree again afterwards. A move
// has no order to walk while a search narrows the list, which canMove answers.
const order = useSessionMove(() => props.hub, () => props.reload(), props.onError, searching);
// Moving a conversation into another project: the composable owns the question,
// its refusal and the re-read; the column owns which row asked it.
const moveTo = useSessionMoveTo({
  hub: props.hub,
  tree: () => props.tree,
  reload: () => props.reload(),
  label: rowLabel,
  addProject: props.onAddWorkspace,
});
// Which conversations have their kept versions and copies drawn inline. It is
// the column's own reading of the tree, not a fact the kernel keeps.
const spread = ref<Set<string>>(new Set());
const keptCount = computed(() => (menuSession.value?.versions?.length ?? 0) + (menuSession.value?.copies?.length ?? 0));
const exporting = ref<string | null>(null);
const naming = ref<string | null>(null);
const menuSessionWorkspace = computed(() => {
  const session = menuSession.value;
  return session ? workspaceOf(props.tree, session.path) ?? null : null;
});

function pinTarget() {
  const session = menuSession.value;
  if (session) void props.hub.pinSession(session.path, !session.pinned).then(() => props.reload()).catch(props.onError);
}

function autoNameTarget() {
  const session = menuSession.value;
  if (!session) return;
  naming.value = session.path;
  void props.hub.autoNameSession(session.path)
    .then(({ title }) => props.hub.renameSession(session.path, title))
    .then(() => props.reload())
    .catch(props.onError)
    .finally(() => { naming.value = null; });
}

function markReadTarget() {
  const session = menuSession.value;
  if (session) void props.hub.markSessionRead(session.path).then(() => props.reload()).catch(props.onError);
}

function copyInfoTarget() {
  const session = menuSession.value;
  if (session) void copyText(sessionInfoText(session, session.workspaceRoot || menuSessionWorkspace.value?.root || "")).catch(props.onError);
}

function exportTarget(format: "json" | "md") {
  const session = menuSession.value;
  if (!session) return;
  exporting.value = session.path;
  void saveSessionFile(props.hub, session, format).catch(props.onError).finally(() => { exporting.value = null; });
}

function moveTarget(direction: -1 | 1) {
  const session = menuSession.value;
  if (session) void order.move(session, direction);
}

function keptTarget() {
  const session = menuSession.value;
  if (!session) return;
  const path = session.path;
  const next = new Set(spread.value);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  spread.value = next;
}

// The question is asked from the row it belongs to: the project it leaves is
// the one the tree files it under, never a guess from the path.
function moveToTarget() {
  const session = menuSession.value;
  const from = session?.workspaceRoot || menuSessionWorkspace.value?.root || "";
  if (session && from) moveTo.request(session, from);
}

// A kept version or copy is a whole conversation the kernel already holds, so
// the kernel's own delete is what removes it, and the tree is read again after.
function deleteKept(path: string) {
  void props.hub.removeSession(path).then(() => props.reload()).catch(props.onError);
}

function archiveWorkspaceTarget() {
  const at = menuAt.value;
  if (at?.of === "workspace") props.onArchiveWorkspace(at.root);
}

function dismissOnPress(event: MouseEvent) {
  if (!menuAt.value) return;
  const at = event.target as Node;
  if (menuElement()?.contains(at) || opener?.contains(at)) return;
  closeMenu(false);
}

function dismissOnMove() {
  if (menuAt.value) closeMenu(false);
}

// Ctrl/Cmd+K is the palette — the one combination this column prints, so it has
// to be the one it answers. Alt+N starts a conversation, which is what the
// button above it says. Both are read here rather than in the child, because a
// column that is mounted but folded out of sight must not answer either.
function onWindowKey(event: KeyboardEvent) {
  const mod = event.ctrlKey || event.metaKey;
  if (mod && !event.altKey && event.key.toLowerCase() === "k") {
    event.preventDefault();
    paletteOpen.value = !paletteOpen.value;
    return;
  }
  if (event.altKey && !mod && event.key.toLowerCase() === "n") {
    event.preventDefault();
    props.onNewSession?.();
    return;
  }
  if (event.key === "Escape" && menuAt.value) {
    event.stopPropagation();
    closeMenu();
  }
}

onMounted(() => {
  addEventListener("mousedown", dismissOnPress);
  addEventListener("resize", dismissOnMove);
  addEventListener("scroll", dismissOnMove, true);
  addEventListener("keydown", onWindowKey);
});

onUnmounted(() => {
  removeEventListener("mousedown", dismissOnPress);
  removeEventListener("resize", dismissOnMove);
  removeEventListener("scroll", dismissOnMove, true);
  removeEventListener("keydown", onWindowKey);
});

/* ── Recent conversations ─────────────────────────────────────────────────── */

// The rows are the tree's own, already read and re-read by the window: asking
// the kernel again would be a second reader of one list with its own staleness.
const allRecent = computed(() => recentRows(props.tree, 0));

function pickRecent(path: string) {
  recentOpen.value = false;
  openSession(path);
}

// A filter that only counts some rows would otherwise land on a column that
// looks unchanged, because every project it found rows in is folded.
watch(scope, (next) => {
  if (next === "all" || searching.value || foldable.value.length === 0) return;
  setFolded(foldedWith(folded.value, foldable.value, false));
});

const workspaceName = (workspace: { name: string; root: string }) => workspace.name || workspace.root;
</script>

<template>
  <aside class="rail" :aria-label="t('app.workspaces')" :aria-busy="openingPending ? 'true' : undefined">
    <div class="railscroll">
      <div class="studio-rail-head">
        <button
          type="button"
          class="studio-new-task"
          data-action="session.new"
          :aria-label="t('common.start_a_new_session')"
          :title="t('common.start_a_new_session')"
          @click="onNewSession?.()"
        >
          <span aria-hidden="true"><StudioIcon name="plus" /></span>
          <b class="studio-label">{{ t("common.start_a_new_session") }}</b>
        </button>
        <SidebarSearch :value="query" :shortcut="shortcutLabel()" @query="query = $event" @palette="paletteOpen = true" />
      </div>

      <div class="projects-header">
        <div class="projects-tabs">
          <div class="vtabs" role="group" :aria-label="t('session.session_view')">
            <button type="button" class="vtab" data-action="session.view" data-value="projects" aria-pressed="true">{{ t("common.project") }}</button>
            <button type="button" class="vtab" data-action="session.recent" :title="t('session.recent_sessions')" :aria-label="t('session.recent_sessions')" @click="recentOpen = true">{{ t("session.recent") }}</button>
          </div>
          <span class="projects-spacer" />
          <button
            type="button"
            class="projects-add projects-fold"
            data-action="workspace.fold-all"
            :disabled="foldable.length === 0 || searching"
            :aria-expanded="!allShut"
            :title="t(searching ? 'session.projects_expand_automatically_while_searching' : allShut ? 'session.expand_all_projects' : 'session.collapse_all_projects')"
            :aria-label="t(allShut ? 'session.expand_all_projects' : 'session.collapse_all_projects')"
            @click="toggleAll"
          ><StudioIcon :name="allShut ? 'expandAll' : 'collapseAll'" /></button>
          <SessionFilter :scope="scope" :counts="counts" :allowed="SCOPES" :names="SCOPE_KEY" @scope="scope = $event" />
          <button
            v-if="onOpenRemotes"
            type="button"
            class="projects-add"
            data-action="remotes.manage"
            :title="t('settings.other_machines')"
            :aria-label="t('settings.other_machines')"
            @click="onOpenRemotes"
          ><StudioIcon name="server" /></button>
          <button
            type="button"
            class="projects-add"
            data-action="workspace.add"
            :disabled="busy"
            :title="t('session.open_project')"
            :aria-label="t('session.open_project')"
            @click="onAddWorkspace"
          ><StudioIcon name="plus" /></button>
        </div>
      </div>

      <ContentMatches :hub="hub" :tree="tree" :needle="query" :on-open="openSession" :on-error="onError" />

      <div
        ref="treeKeys.ref"
        class="scroll"
        role="tree"
        :aria-label="t('session.projects_and_conversations')"
        data-action-keydown="tree.navigate"
        @keydown="treeKeys.onKeyDown"
      >
        <div v-if="empty" class="sessnote" role="status">
          <p>{{ t("session.no_folders_yet") }}</p>
          <button type="button" class="btn" data-action="workspace.add" @click="onAddWorkspace">{{ t("session.open_your_first_project") }}</button>
        </div>
        <div v-else-if="visible.length === 0" class="sessnote" role="status">
          <p>{{ t("common.nothing_matches") }}</p>
          <button type="button" class="btn" data-action="workspace.search" data-value="clear" @click="query = ''; scope = 'all'">{{ t("session.clear_filter") }}</button>
        </div>
        <WorkspaceGroup
          v-for="workspace in visible"
          :key="workspace.root"
          :root="workspace.root"
          :name="workspace.name"
          :parent="parentName(workspace.root)"
          :isolated="workspace.isolated"
          :extra="workspace.workspaceRoots?.length ?? 0"
          :foldable="workspace.sessions.length > 0"
          :shut="isShut(folded, workspace.root, searching)"
          :more="workspace.remaining ?? 0"
          :unread="workspace.sessions.filter((row) => row.unread).length"
          :rows="workspace.sessions"
          :needle="query.trim().toLowerCase()"
          :active-path="activeSessionPath"
          :opening="opening"
          :spread="spread"
          :busy="!!busy"
          :locked="!!workspace.missing"
          :on-fold="fold"
          :on-new-session="newSession"
          :on-menu="openWorkspaceMenu"
          :on-open="openSession"
          :on-rename="onRenameSession"
          :on-archive="onArchiveSession"
          :on-delete="onDeleteSession"
          :on-delete-kept="deleteKept"
          :on-session-menu="openSessionMenu"
        />
      </div>

      <div v-if="previewRows.length > 0" class="railfoot">
        <div class="studio-section-label studio-session-label">
          <span>{{ t("session.recent_sessions") }}</span>
          <button type="button" data-action="session.recent" :title="t('session.recent_sessions')" :aria-label="t('session.recent_sessions')" @click="recentOpen = true"><StudioIcon name="external" /></button>
        </div>
        <button
          v-for="row in previewRows"
          :key="row.session.path"
          type="button"
          class="ds-row"
          data-action="session.open"
          :data-target="row.session.path"
          :title="`${workspaceName(row.workspace)} · ${rowLabel(row.session)}`"
          @click="openSession(row.session.path)"
        >
          <span class="ds-lead"><i class="pip" /></span>
          <span class="ds-title">{{ rowLabel(row.session) }}</span>
          <span class="ds-meta">{{ workspaceName(row.workspace) }}</span>
        </button>
      </div>
    </div>

    <Teleport to="body">
      <SessionMenu
        v-if="menuSession"
        ref="sessionMenu"
        :label="rowLabel(menuSession)"
        :path="menuSession.path"
        :archived="!!menuSession.archived"
        :held="menuHeld"
        :live="menuLive"
        :pinned="!!menuSession.pinned"
        :exporting="exporting === menuSession.path"
        :auto-naming="naming === menuSession.path"
        :can-rename="!!onRenameSession"
        :can-move="menuSessionWorkspace ? order.canMove(menuSessionWorkspace, menuSession) : undefined"
        :on-rename="() => take(renameTarget)"
        :on-archive="() => take(archiveTarget)"
        :on-delete="() => take(deleteTarget)"
        :on-pin="() => take(pinTarget)"
        :on-auto-name="() => take(autoNameTarget)"
        :on-mark-read="menuSession.unread ? () => take(markReadTarget) : undefined"
        :on-copy-info="() => take(copyInfoTarget)"
        :on-export="(format) => take(() => exportTarget(format))"
        :on-move="(direction) => take(() => moveTarget(direction))"
        :kept-count="keptCount"
        :kept-open="spread.has(menuSession.path)"
        :on-kept="() => take(keptTarget)"
        :on-move-to="() => take(moveToTarget)"
        :on-pause="onPauseSession ? () => take(pauseTarget) : undefined"
        :on-close-conversation="onCloseSession ? () => take(closeTarget) : undefined"
        :on-branch="onBranchSession && menuSession.runtimeId ? () => take(branchTarget) : undefined"
        @close="closeMenu()"
      />
      <WorkspaceMenu
        v-else-if="menuWorkspace"
        ref="workspaceMenu"
        :name="menuWorkspace.name || menuWorkspace.root"
        :root="menuWorkspace.root"
        :on-archive="() => take(archiveWorkspaceTarget)"
        @close="closeMenu()"
      />
    </Teleport>

    <MoveSessionDialog v-if="moveTo.dialog.value" v-bind="moveTo.dialog.value" />

    <Palette
      :open="paletteOpen"
      :commands="commands"
      :tree="visible"
      :on-open-session="openSession"
      :on-close="() => { paletteOpen = false; }"
    />
    <RecentSessions
      v-if="recentOpen"
      :rows="allRecent"
      :workspace-name="workspaceName"
      :on-pick="pickRecent"
      :on-close="() => { recentOpen = false; }"
    />
  </aside>
</template>
