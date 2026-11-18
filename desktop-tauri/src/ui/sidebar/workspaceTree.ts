import type { HubPort, TreeSession, TreeWorkspace } from "../../port/hub";
import { t } from "../../i18n";
import { inScope, type SessionScope } from "../sessionScope";

/** What the window hands the workspace column.
 *
 *  Declared here rather than inside the component so the window can name the
 *  contract without importing the column, and so the column's own modules can
 *  type their props against the same declaration. */
export interface SidebarProps {
  hub: HubPort;
  tree: TreeWorkspace[];
  /** The runtime id of the focused pane. */
  active: string;
  /** The conversation that pane drives, when it drives one. */
  activeSessionPath?: string;
  busy?: boolean;
  onOpenSession: (path: string) => void;
  onNewSession: () => void;
  onAddWorkspace: () => void;
  onArchiveWorkspace: (root: string) => void;
  onRenameSession?: (path: string, title: string) => void;
  onArchiveSession?: (path: string) => void;
  onDeleteSession?: (path: string) => void;
  /** Each open runtime's running state, keyed by runtime id: the rail gates a
   *  conversation's pause/close/archive on its own pane, not the focused one. */
  runs?: Record<string, { live: boolean }>;
  onPauseSession?: (runtimeId: string) => void;
  onCloseSession?: (runtimeId: string) => void;
  onBranchSession?: (runtimeId: string, name: string) => Promise<void>;
  /** The window's tree re-read, asked after a mutation the column runs itself
   *  (pin, name, read, move): the kernel owns the list, so its answer is the row
   *  worth drawing afterwards. */
  reload: () => Promise<void>;
  onOpenSettings?: (at?: string) => void;
  /** The machines panel. Absent on a kernel that does not do remote panes, which
   *  is the same answer remoteHosts() gives: no entry, because nothing it offers
   *  could work here. */
  onOpenRemotes?: () => void;
  onError: (e: unknown) => void;
}

/** The scopes the filter offers. `waiting` is left out: whether a run is blocked
 *  on a person is the pane's reading, and this column only receives the tree. */
export const SCOPES: readonly SessionScope[] = ["all", "live", "unread", "pinned", "archived"];

/** The message a scope is named by. Keys rather than sentences, so the menu and
 *  the trigger cannot drift into two spellings of the same choice. */
export const SCOPE_KEY: Record<SessionScope, string> = {
  all: "common.all",
  // The tree says which conversations a pane drives, not which are mid-turn;
  // "in progress" here means exactly that much and no more.
  live: "plan.step.now",
  waiting: "chat.needs_you",
  unread: "common.unread",
  pinned: "session.pinned_2",
  archived: "common.archived",
};

const RECENT_LIMIT = 5;

/** A conversation a pane holds before its first turn is written has only a file
 *  name; calling it that would show a timestamp where every other row shows words. */
export function rowLabel(session: TreeSession): string {
  return session.title || (session.runtimeId && !session.turns ? t("common.new_session") : session.name) || session.path;
}

const SEP = /[/\\]+/;

export function baseName(path: string): string {
  return path.split(SEP).filter(Boolean).at(-1) || path;
}

/** The folder above a project, which is what tells two projects of the same name
 *  apart. The whole path is on the header title and in its menu. */
export function parentName(path: string): string {
  const parts = path.split(SEP).filter(Boolean);
  return parts.length >= 2 ? parts[parts.length - 2] : "";
}

/** Two segments of a path, elided from the left: the name is the identity and
 *  the whole path is one hover away. */
export function compactPath(path: string, segments = 2): string {
  const parts = path.split(SEP).filter(Boolean);
  return parts.length > segments ? "…/" + parts.slice(-segments).join("/") : path;
}

export function matchesNeedle(hay: string, needle: string): boolean {
  const q = needle.trim().toLowerCase();
  return !q || hay.toLowerCase().includes(q);
}

/** The kernel's own order with pinned rows led. It is not re-sorted by time:
 *  every other reader of this tree draws it in the order the kernel chose, and a
 *  column that disagreed would put the same conversation in two places. */
export function orderSessions(sessions: readonly TreeSession[]): TreeSession[] {
  const lead = sessions.filter((session) => session.pinned);
  if (lead.length === 0 || lead.length === sessions.length) return [...sessions];
  return [...lead, ...sessions.filter((session) => !session.pinned)];
}

/** The projects and rows the column draws for one search word and one scope.
 *
 *  A project whose name matches keeps all of its rows: someone typing a project
 *  name is asking for the project, not for the two conversations that happen to
 *  repeat the word. A project left with no rows is dropped rather than drawn as
 *  an empty heading. An archived row answers a search — that is what a search
 *  is for — and is otherwise in no list but Archived. */
export function filterTree(tree: readonly TreeWorkspace[], needle: string, scope: SessionScope): TreeWorkspace[] {
  const q = needle.trim().toLowerCase();
  const out: TreeWorkspace[] = [];
  for (const workspace of tree) {
    const named = !!q && matchesNeedle(`${workspace.name} ${workspace.root}`, q);
    const rows = (named ? [...workspace.sessions] : workspace.sessions.filter((session) => matchesNeedle(rowLabel(session), q)))
      .filter((session) => inScope(session, scope, !!q, !!session.runtimeId, !!session.pinned));
    if (!named && rows.length === 0) continue;
    out.push({ ...workspace, sessions: orderSessions(rows) });
  }
  return out;
}

/** What each choice in the filter is worth, counted once so the trigger and the
 *  menu cannot disagree about the same number. */
export function scopeCounts(tree: readonly TreeWorkspace[]): Record<SessionScope, number> {
  const all = tree.flatMap((workspace) => workspace.sessions);
  const open = all.filter((session) => !session.archived);
  return {
    all: open.length,
    live: open.filter((session) => !!session.runtimeId).length,
    waiting: 0,
    unread: open.filter((session) => !!session.unread).length,
    pinned: open.filter((session) => !!session.pinned).length,
    archived: all.length - open.length,
  };
}

/** The projects a fold applies to. A project with nothing in it has nothing to
 *  fold, and a collapsed heading over it would only hide that it is empty. */
export function foldableRoots(tree: readonly TreeWorkspace[]): string[] {
  return tree
    .filter((workspace) => workspace.sessions.length > 0 || (workspace.total ?? 0) > 0 || workspace.listingState === "degraded")
    .map((workspace) => workspace.root);
}

export function allFolded(folded: ReadonlySet<string>, roots: readonly string[]): boolean {
  return roots.length > 0 && roots.every((root) => folded.has(root));
}

/** A new fold set with one decision applied to several projects at once. The
 *  caller's set is never written to: it is also the persisted one. */
export function foldedWith(folded: ReadonlySet<string>, roots: readonly string[], shut: boolean): Set<string> {
  const next = new Set(folded);
  for (const root of roots) {
    if (shut) next.add(root);
    else next.delete(root);
  }
  return next;
}

/** Whether a project's rows are hidden right now. A search opens everything: a
 *  fold is a resting-state preference, and it would hide the very rows the word
 *  just found. */
export function isShut(folded: ReadonlySet<string>, root: string, searching: boolean): boolean {
  return !searching && folded.has(root);
}

export interface RecentRow {
  workspace: TreeWorkspace;
  session: TreeSession;
}

/** The conversations said in most recently, across every project. Archived ones
 *  are left out: this is the list someone reaches for to carry on working. */
export function recentRows(tree: readonly TreeWorkspace[], limit = RECENT_LIMIT): RecentRow[] {
  const rows = tree.flatMap((workspace) => workspace.sessions
    .filter((session) => !session.archived)
    .map((session) => ({ workspace, session })));
  rows.sort((a, b) => (b.session.lastActiveAt ?? 0) - (a.session.lastActiveAt ?? 0) || a.session.path.localeCompare(b.session.path));
  return limit > 0 ? rows.slice(0, limit) : rows;
}

/** The project a conversation belongs to, found by the row itself rather than by
 *  an index the window would have to keep in step with the tree. */
export function workspaceOf(tree: readonly TreeWorkspace[], path: string | undefined): TreeWorkspace | undefined {
  if (!path) return undefined;
  for (const workspace of tree) {
    if (workspace.sessions.some((session) => session.path === path)) return workspace;
  }
  return undefined;
}
