<script lang="ts">
import type { AgentPort } from "../../port/port";

export interface WorkbenchPanelProps {
  port: AgentPort;
  /** The workspace the explorer, the editor and the changes are read from. */
  root: string;
  onError: (e: unknown) => void;
  /** Puts the whole panel away; the pane that holds it decides what that means. */
  onClose: () => void;
  visible: boolean;
  /** A request to open sub-agent runs or background jobs as tabs. */
  agentRequest?: AgentRequest;
  onShowCall?: (callId: string) => void;
  /** A file link in the conversation, to be taken once. */
  fileRequest?: { path: string; consumed: () => void };
  onActiveFile?: (path: string | undefined) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason, WORKSPACE_FILE_MISSING } from "../../i18n/kernel";
import { host } from "../../port/host";
import { HttpError } from "../../port/http_error";
import type { WorkspaceChange, WorkspaceFile } from "../../port/port";
import { setShowsHiddenFiles, showsHiddenFiles } from "../../state/prefs";
import { locate } from "../../state/gittree";
import { useGlance } from "../glance";
import { parseGrantedFile, documentRefs } from "../grantedfiles";
import { pinToViewport } from "../place";
import { readRecentFiles, rememberFile } from "../recentFiles";
import ExplorerResize from "../ExplorerResize.vue";
import LazyMarkdown from "../LazyMarkdown.vue";
import StudioIcon, { type StudioIconName } from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";
import UnifiedDiff from "../UnifiedDiff.vue";
import { useCommitCard } from "../CommitCard";
import { useGitRepos } from "../useGitRepos";
import { usePushCard } from "../GitPushCard";
import { useTerminalTabs } from "../useTerminalTabs";
import { useAgentTabs, type AgentRequest } from "../useAgentTabs";
import AgentRunView from "../AgentRunView.vue";
import JobRunView from "../JobRunView.vue";
import { keyOf, labelOf, type Surface } from "../workbench_tabs";
import { treeRows } from "../workbench_tree";
import type { TextMenuAction } from "../workbench_text_menu";
import WorkbenchChanges from "./WorkbenchChanges.vue";
import WorkbenchExplorerHead from "../WorkbenchExplorerHead.vue";
import WorkbenchFileBar, { type FileMode } from "../WorkbenchFileBar.vue";
import { documentView, sourceView } from "../document/registry";
import { documentState, type DocumentViewState } from "../document/viewState";
import DocumentPreview from "../document/DocumentPreview.vue";
import WorkbenchFileMenu, { type FileMenu } from "./WorkbenchFileMenu.vue";
import WorkbenchFileTree from "../WorkbenchFileTree.vue";
import WorkbenchGit from "./WorkbenchGit.vue";
import WorkbenchSelectionMenu from "../WorkbenchSelectionMenu.vue";
import WorkbenchState from "../WorkbenchState.vue";
import WorkbenchTabOverflow from "../WorkbenchTabOverflow.vue";
import WorkbenchTerminals from "../terminal/WorkbenchTerminals.vue";
import type { TextContextMenuRequest } from "../../port/host";

const props = defineProps<WorkbenchPanelProps>();

const showFiles = ref(true);
const query = ref("");
const hidden = ref(showsHiddenFiles());
const collapsed = ref<Set<string>>(new Set());
const files = ref<string[]>([]);
const directories = ref<string[]>([]);
const listFailed = ref("");
const failed = ref("");
const changesOpen = ref(false);
const openFiles = ref<string[]>([]);
const selected = ref("");
const platform = ref("");
// The terminal draws in the page's own palette, so the scheme it is given is
// the page's rather than a second copy of the setting. The theme is one
// attribute away, and it is observed rather than polled.
const readScheme = (): "light" | "dark" => (document.documentElement.dataset.theme === "light" ? "light" : "dark");
const scheme = ref<"light" | "dark">(readScheme());
let watchingTheme: MutationObserver | null = null;
const revealable = ref(false);
const changes = ref<WorkspaceChange[]>([]);
const menu = ref<(FileMenu & { id: number }) | null>(null);
const textMenu = ref<TextContextMenuRequest | null>(null);
const asking = ref<{ path: string; anchor: HTMLElement } | null>(null);
const askBox = ref<HTMLElement | null>(null);
const cursor = ref<[string, number]>(["", 1]);
const strip = ref<HTMLDivElement | null>(null);
const dismissed = ref<Set<string>>(new Set());
const overflowKeys = ref<string[]>([]);
const openFolders = ref<string[]>([]);
let menuSerial = 0;
let treeTicket = 0;

const glance = useGlance();
const gitRepos = useGitRepos(() => props.port, glance);
const terminal = useTerminalTabs({
  port: () => props.port,
  shown: () => props.visible,
  pick: (key) => { selected.value = key; },
  setDismissed: (next) => { dismissed.value = typeof next === "function" ? next(dismissed.value) : next; },
  onExternal: (url) => void props.port.openExternal(url).catch((e: unknown) => props.onError(e)),
  onError: (message) => props.onError(new Error(message)),
  request: ref(0),
});

// What differs in the workspace's own repository, read when the panel opens and
// again when the window comes back into view: another application's writes
// announce nothing to this one.
const readChanges = () => {
  void props.port.changes().then((r) => { changes.value = r.changes; }, () => { changes.value = []; });
};
watch([() => props.port, () => props.root, glance], readChanges, { immediate: true });

const changeKey = computed(() => changes.value.map((c) => `${c.status}:${c.path}`).join("\n"));

// The tree is read when the panel is shown and whenever the disk may have
// moved: a change set that differs, a window that came back, a reload.
const readTree = async () => {
  const ticket = ++treeTicket;
  const live = () => ticket === treeTicket;
  try {
    const top = await props.port.workspaceFiles("", query.value, hidden.value);
    let found = top.files;
    let dirs = top.directories;
    // A reload is the tree read again, not the reader's place in it lost: the
    // folders they had open are read again, parents before children.
    const open = new Set<string>();
    if (!query.value) {
      const depth = (path: string) => path.split("/").length;
      for (const path of [...openFolders.value].sort((a, b) => depth(a) - depth(b))) {
        if (!dirs.includes(path)) continue;
        try {
          const inner = await props.port.workspaceFiles(path, "", hidden.value);
          found = [...found, ...inner.files];
          dirs = [...dirs, ...inner.directories];
          open.add(path);
        } catch {
          // Left closed; the next click says why.
        }
      }
    }
    if (!live()) return;
    files.value = [...new Set(found)];
    directories.value = [...new Set(dirs)];
    collapsed.value = query.value ? new Set() : new Set(dirs.filter((path) => !open.has(path)));
    openFolders.value = [...open];
    listFailed.value = "";
  } catch (e) {
    if (live()) listFailed.value = reason(e);
  }
};

watch([() => props.port, () => props.root, () => props.visible, query, hidden, changeKey, glance], () => {
  if (props.visible) void readTree();
}, { immediate: true });

onMounted(() => {
  revealable.value = props.port.revealsFiles();
  void host().describe().then((h) => { platform.value = h.platform; }, () => {});
  if (typeof MutationObserver === "function") {
    watchingTheme = new MutationObserver(() => { scheme.value = readScheme(); });
    watchingTheme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  }
});

const rows = computed(() => treeRows(files.value, directories.value, query.value, collapsed.value));

const toggleFolder = async (path: string) => {
  if (!collapsed.value.has(path)) {
    const next = new Set(collapsed.value).add(path);
    collapsed.value = next;
    return;
  }
  try {
    const result = await props.port.workspaceFiles(path, "", hidden.value);
    files.value = [...new Set([...files.value, ...result.files])];
    directories.value = [...new Set([...directories.value, ...result.directories])];
    const next = new Set(collapsed.value);
    next.delete(path);
    result.directories.forEach((dir) => next.add(dir));
    collapsed.value = next;
    openFolders.value = [...openFolders.value, path];
    listFailed.value = "";
  } catch (e) {
    listFailed.value = reason(e);
  }
};

// One buffer per open path: closing a tab keeps what was typed, so a file put
// away mid-edit comes back as it was left rather than as the disk has it.
interface Buffer {
  file: WorkspaceFile | null;
  draft: string;
  diff: string;
  truncated: boolean;
  error: string;
  missing: boolean;
  loading: boolean;
}
// Reactive, so typing marks the tab and enables Save: a plain Map would hold
// the same object every read and no consumer would ever be told it changed.
const buffers = reactive(new Map<string, Buffer>());
const blank = (): Buffer => ({ file: null, draft: "", diff: "", truncated: false, error: "", missing: false, loading: false });
/** The buffer for a path, made and published on first use. Only a read or a
 *  keystroke calls this: a computed that published one would be writing state
 *  from inside a render. */
const ensure = (path: string): Buffer => {
  const held = buffers.get(path);
  if (held) return held;
  const fresh = blank();
  buffers.set(path, fresh);
  return fresh;
};
const bufferOf = (path: string): Buffer => buffers.get(path) ?? blank();
const dirtyOf = (path: string) => {
  const entry = buffers.get(path);
  return !!entry?.file && entry.draft !== entry.file.content;
};

const terminalTabs = computed(() => terminal.tabs.value);
// A closed terminal is dismissed, not reopened: its tab leaves and so does its
// screen, so the layer is handed only the ones still open.
const liveTerminals = computed(() => terminal.terminals.value.filter((info) => !dismissed.value.has(`terminal:${info.id}`)));
const surfaces = computed<Surface[]>(() => [
  ...(changesOpen.value ? [{ kind: "changes" } as const] : []),
  ...openFiles.value.map((path) => ({ kind: "file", path }) as const),
  ...terminalTabs.value,
  ...agents.value,
].filter((s) => !dismissed.value.has(keyOf(s))));

const active = computed<Surface | undefined>(() => surfaces.value.find((s) => keyOf(s) === selected.value) ?? surfaces.value[0]);
const activeKey = computed(() => (active.value ? keyOf(active.value) : ""));
const filePath = computed(() => (active.value?.kind === "file" ? active.value.path : ""));
const held = computed(() => parseGrantedFile(filePath.value));
const buffer = computed<Buffer>(() => (filePath.value ? bufferOf(filePath.value) : blank()));

const labelFor = (surface: Surface) => labelOf(surface, {});

const iconOf = (surface: Surface): StudioIconName => {
  switch (surface.kind) {
    case "changes": return "branch";
    case "terminal": return "terminal";
    case "browser":
    case "manual": return "globe";
    case "agent": return "agent";
    case "job": return "play";
    default: return "file";
  }
};

const pick = (key: string) => {
  if (!key.startsWith("file:")) showFiles.value = false;
  failed.value = "";
  selected.value = key;
};

const { agents, setAgents } = useAgentTabs(() => props.agentRequest, pick, (next) => {
  dismissed.value = typeof next === "function" ? next(dismissed.value) : next;
});

const readBuffer = async (path: string) => {
  if (!path) return;
  const entry = ensure(path);
  const granted = parseGrantedFile(path);
  entry.loading = true;
  if (!entry.file) entry.error = "";
  try {
    const file = granted
      ? await props.port.workspaceFile(granted.rel, granted.root)
      : await props.port.workspaceFile(path);
    if (dirtyOf(path)) {
      // Typed while this read was in flight: the text on screen is the one the
      // reader meant, so only the identity of the file is taken.
      entry.file = { ...file, content: entry.file?.content ?? file.content };
    } else {
      entry.file = file;
      entry.draft = file.content;
    }
    entry.error = "";
    entry.missing = false;
    if (!granted) {
      try {
        const at = locate(gitRepos.repos.value, path);
        const d = await props.port.changeDiff(at.rel, at.repo || undefined);
        entry.diff = d.diff;
        entry.truncated = d.truncated;
      } catch {
        // A path git does not track has no diff; it is still readable.
        entry.diff = "";
        entry.truncated = false;
      }
    }
  } catch (e) {
    entry.missing = e instanceof HttpError && e.reason?.code === WORKSPACE_FILE_MISSING;
    entry.error = reason(e);
    entry.file = null;
    entry.draft = "";
  } finally {
    entry.loading = false;
  }
};

watch([filePath, () => props.visible], ([path, shown]) => {
  if (shown && path) void readBuffer(path);
}, { immediate: true });

// A file that opened rendered stays rendered: it is read far more often than
// it is edited, and its source view is one click away.
const readable = computed(() => /\.(md|markdown|mdx)$/i.test(held.value?.rel ?? filePath.value));
const hasDocument = computed(() => documentView(held.value?.rel ?? filePath.value));
const source = computed(() => sourceView(held.value?.rel ?? filePath.value));
const mode = ref<FileMode>("file");
watch(filePath, () => {
  mode.value = mode.value === "diff" && source.value ? "diff" : hasDocument.value ? "document" : readable.value ? "read" : "file";
}, { immediate: true });

// Each open file keeps its own scroll/zoom, so returning to a preview lands
// where it was left rather than at the top. Pruned to the tabs still open.
const documentStates = new Map<string, DocumentViewState>();
watch(openFiles, (paths) => {
  for (const path of documentStates.keys()) if (!paths.includes(path)) documentStates.delete(path);
});
const documentRefresh = computed(() => JSON.stringify([changeKey.value, glance.value]));
const openDocument = () => {
  const path = held.value ? `${held.value.root}/${held.value.rel}` : filePath.value;
  void props.port.openDocument(path).catch((e: unknown) => props.onError(e));
};
const canOpenDocument = computed(() => props.port.opensFiles() && /\.(pdf|html?|svg|docx?|xlsx?|pptx?)$/i.test(filePath.value));

const setDraft = (next: string) => {
  const entry = filePath.value ? buffers.get(filePath.value) : undefined;
  if (!entry?.file || entry.file.readOnly) return;
  entry.draft = next;
};

const save = async (): Promise<boolean> => {
  const path = filePath.value;
  const entry = buffers.get(path);
  if (!entry?.file || entry.file.readOnly || entry.draft === entry.file.content) return true;
  entry.loading = true;
  try {
    entry.file = await props.port.saveWorkspaceFile({ ...entry.file, content: entry.draft });
    entry.error = "";
    readChanges();
    void readTree();
    return true;
  } catch (e) {
    entry.error = reason(e);
    return false;
  } finally {
    entry.loading = false;
  }
};

const openFile = (path: string) => {
  if (!openFiles.value.includes(path)) openFiles.value = [...openFiles.value, path];
  recentFiles.value = rememberFile(props.root, path);
  dismissed.value = new Set([...dismissed.value].filter((k) => k !== `file:${path}`));
  selected.value = `file:${path}`;
};

watch(filePath, (path) => props.onActiveFile?.(path || undefined), { immediate: true });

const readRefs = computed(() => (filePath.value ? documentRefs(filePath.value, props.port, openFile) : undefined));

const recentFiles = ref<string[]>([]);
watch(() => props.root, (root) => { recentFiles.value = readRecentFiles(root); }, { immediate: true });

watch(() => props.fileRequest, (request) => {
  if (!request) return;
  openFile(request.path);
  request.consumed();
}, { immediate: true });

const openChanges = () => {
  changesOpen.value = true;
  selected.value = "changes:";
};

const closeSurface = async (surface: Surface) => {
  if (surface.kind === "file") {
    if (dirtyOf(surface.path)) {
      const anchor = document.querySelector<HTMLElement>(`[data-key="file:${CSS.escape(surface.path)}"] .workbench-tab-close`) ?? document.body;
      asking.value = { path: surface.path, anchor };
      return;
    }
    openFiles.value = openFiles.value.filter((p) => p !== surface.path);
    if (activeKey.value === keyOf(surface)) selected.value = "";
    return;
  }
  if (surface.kind === "changes") {
    changesOpen.value = false;
    if (activeKey.value === "changes:") selected.value = "";
    return;
  }
  if (surface.kind === "agent" || surface.kind === "job") {
    setAgents((prev) => prev.filter((tab) => keyOf(tab) !== keyOf(surface)));
    if (activeKey.value === keyOf(surface)) selected.value = "";
    return;
  }
  if (surface.kind === "terminal") {
    await terminal.guard([surface], async (targets) => {
      for (const target of targets) {
        if (target.kind !== "terminal") continue;
        try {
          await props.port.closeTerminal(target.id);
        } catch (e) {
          failed.value = t("workbench.could_not_close_name_why", { name: target.name, why: reason(e) });
        }
        dismissed.value = new Set([...dismissed.value, `terminal:${target.id}`]);
      }
    });
    if (activeKey.value === keyOf(surface)) selected.value = "";
  }
};

const dropUnsaved = () => {
  const at = asking.value?.path;
  asking.value = null;
  if (!at) return;
  const entry = buffers.get(at);
  if (entry?.file) entry.draft = entry.file.content;
  openFiles.value = openFiles.value.filter((p) => p !== at);
  if (selected.value === `file:${at}`) selected.value = "";
};

const saveUnsaved = async () => {
  const at = asking.value?.path;
  if (!at) return;
  // A tab that is not open was already dropped, so the buffer is still there
  // to be written; the file is put in front because save() writes what is.
  selected.value = `file:${at}`;
  await nextTick();
  const ok = await save();
  if (!ok) return;
  asking.value = null;
  openFiles.value = openFiles.value.filter((p) => p !== at);
  if (selected.value === `file:${at}`) selected.value = "";
};

const reveal = (path: string) => {
  menu.value = null;
  void props.port.revealInFileManager(path).then(() => { failed.value = ""; }, (e: unknown) => { failed.value = reason(e); });
};

const offerMenu = (e: MouseEvent, path: string, folder: boolean) => {
  e.preventDefault();
  if (document.getSelection()?.isCollapsed === false) return;
  const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const keyed = e.clientX === 0 && e.clientY === 0;
  menu.value = {
    path, folder, x: keyed ? box.left + 24 : e.clientX, y: keyed ? box.bottom : e.clientY,
    anchor: e.currentTarget as HTMLElement, id: ++menuSerial,
  };
};

const moreFromBar = (anchor: HTMLElement) => {
  const box = anchor.getBoundingClientRect();
  menu.value = { path: filePath.value, folder: false, x: box.right - 220, y: box.bottom + 4, anchor, id: ++menuSerial };
};

// Ctrl/Cmd+S saves; the editor is a plain text field, so the shortcut is the
// only way to ask without leaving the keyboard.
const editorKey = (event: Event) => {
  const e = event as KeyboardEvent;
  if (e.type === "keydown" && (e.ctrlKey || e.metaKey) && e.key?.toLowerCase() === "s") {
    e.preventDefault();
    void save();
  }
  const el = event.currentTarget as HTMLTextAreaElement;
  const before = el.value.slice(0, el.selectionStart).split("\n").length;
  cursor.value = [filePath.value, before];
};

const editorMenu = (e: MouseEvent) => {
  const el = e.currentTarget as HTMLTextAreaElement;
  const from = el.selectionStart;
  const to = el.selectionEnd;
  e.preventDefault();
  textMenu.value = { editable: true, selected: to > from, canUndo: false, canRedo: false, quote: false, x: e.clientX, y: e.clientY };
};

const runTextMenu = async (action: TextMenuAction | "quote") => {
  const el = document.querySelector<HTMLTextAreaElement>(".workbench-code");
  if (!el) return;
  if (action === "selectAll") el.select();
  else if (action === "copy") await navigator.clipboard.writeText(el.value.slice(el.selectionStart, el.selectionEnd));
  else if (action === "cut") {
    await navigator.clipboard.writeText(el.value.slice(el.selectionStart, el.selectionEnd));
    setDraft(el.value.slice(0, el.selectionStart) + el.value.slice(el.selectionEnd));
  } else if (action === "paste") {
    const text = await navigator.clipboard.readText();
    setDraft(el.value.slice(0, el.selectionStart) + text + el.value.slice(el.selectionEnd));
  }
  closeTextMenu();
};

const closeTextMenu = () => { textMenu.value = null; };

// A question bubble is fixed, so it is placed beside its anchor rather than
// left to the static position — which is nowhere near the tab it is about.
watch(asking, async (held) => {
  if (!held) return;
  await nextTick();
  const el = askBox.value;
  if (!el) return;
  const at = held.anchor.getBoundingClientRect();
  pinToViewport(el, at.left, at.bottom + 4);
  el.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
});

const dismiss = () => { menu.value = null; };

// Tab overflow: the ones the strip cannot show, in strip order.
const measure = () => {
  const el = strip.value;
  if (!el) return;
  const box = el.getBoundingClientRect();
  const out = [...el.querySelectorAll<HTMLElement>("[data-key]")]
    .filter((tab) => { const r = tab.getBoundingClientRect(); return r.width > 0 && (r.right > box.right + 1 || r.left < box.left - 1); })
    .map((tab) => tab.dataset.key!)
    .filter(Boolean);
  overflowKeys.value = overflowKeys.value.join("\n") === out.join("\n") ? overflowKeys.value : out;
};
const overflowTabs = computed(() => overflowKeys.value.flatMap((key) => {
  const surface = surfaces.value.find((s) => keyOf(s) === key);
  return surface ? [{ key, label: labelFor(surface) }] : [];
}));

let observer: ResizeObserver | null = null;
onMounted(() => {
  measure();
  if (typeof ResizeObserver === "function" && strip.value) {
    observer = new ResizeObserver(measure);
    observer.observe(strip.value);
  }
});
onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
  watchingTheme?.disconnect();
  watchingTheme = null;
});
watch([() => surfaces.value.length, () => props.visible], () => { measure(); });

const scrollStrip = (e: WheelEvent) => {
  if (e.deltaY && strip.value) strip.value.scrollLeft += e.deltaY;
};

// Keep the shown tab in the strip: a selected tab nobody can see reads as a
// tab that closed.
watch(activeKey, (key) => {
  strip.value?.querySelector(`[data-key="${CSS.escape(key)}"]`)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
});

const toggleFiles = () => {
  showFiles.value = !showFiles.value;
  if (showFiles.value) queueMicrotask(() => void readTree());
};

const toggleHidden = (on: boolean) => {
  setShowsHiddenFiles(on);
  hidden.value = on;
};

const revealRoot = () => reveal("");

// The changes tab carries its own commit and push: it is the whole-column view
// of what the explorer's Git section shows in a narrow one.
const commit = useCommitCard(props.port, () => { readChanges(); void readTree(); });
const push = usePushCard(props.port, undefined, () => { readChanges(); });
// Save is offered only for a real edit of a writable file: a granted folder's
// file, or one whose read has not come back, has nothing to write.
const canSave = computed(() => !buffer.value.loading && !buffer.value.file?.readOnly && !!buffer.value.file && buffer.value.draft !== buffer.value.file.content);
</script>

<template>
  <section v-show="visible" class="workbench" :aria-label="t('common.workbench')" data-action-keydown="layer.dismiss">
    <header class="workbench-tabs">
      <TooltipButton
        class="workbench-files"
        data-action="workbench.files"
        :aria-pressed="showFiles"
        :aria-label="t('common.files')"
        :title="showFiles ? t('workbench.hide_file_tree') : t('workbench.show_file_tree')"
        @click="toggleFiles"
      >
        <StudioIcon name="folder" />
        <small v-if="changes.length > 0" class="workbench-files-count">{{ changes.length }}</small>
      </TooltipButton>
      <div ref="strip" class="workbench-tablist" role="tablist" @wheel="scrollStrip">
        <div
          v-for="surface in surfaces"
          :key="keyOf(surface)"
          class="workbench-tab"
          role="tab"
          :aria-selected="keyOf(surface) === activeKey"
          :data-key="keyOf(surface)"
          :data-dirty="surface.kind === 'file' && dirtyOf(surface.path) ? '' : undefined"
        >
          <button
            class="workbench-tab-pick"
            type="button"
            data-action-click="workbench.tab"
            :data-target="keyOf(surface)"
            :title="labelFor(surface)"
            @click="pick(keyOf(surface))"
          >
            <StudioIcon :name="iconOf(surface)" />
            <span>{{ labelFor(surface) }}</span>
            <i v-if="surface.kind === 'terminal' && surface.busy" class="workbench-tab-busy" aria-hidden="true" />
            <small v-if="surface.kind === 'changes' && changes.length > 0" class="workbench-tab-count">{{ changes.length }}</small>
          </button>
          <button
            class="workbench-tab-close"
            type="button"
            data-action="workbench.close"
            :data-target="keyOf(surface)"
            :aria-label="t('common.disable_name', { name: labelFor(surface) })"
            :title="t('common.close')"
            @click="void closeSurface(surface)"
          >
            <StudioIcon name="close" />
          </button>
        </div>
      </div>
      <WorkbenchTabOverflow :tabs="overflowTabs" :on-pick="pick" />
      <TooltipButton class="workbench-new" data-action="workbench.launch-terminal" :title="t('terminal.open')" :aria-label="t('terminal.open')" @click="void terminal.open()">
        <StudioIcon name="plus" />
      </TooltipButton>
      <TooltipButton class="workbench-dismiss" data-action="workbench.dismiss" :title="t('app.close_workbench')" :aria-label="t('app.close_workbench')" @click="onClose">
        <StudioIcon name="close" />
      </TooltipButton>
    </header>

    <div v-if="failed" class="workbench-error" role="alert">{{ failed }}</div>

    <div class="workbench-body" :data-files="showFiles ? '' : undefined">
      <main class="workbench-canvas">
        <div v-if="filePath && buffer.error && !buffer.missing" class="workbench-error" role="alert">{{ buffer.error }}</div>

        <WorkbenchState
          v-if="!active"
          icon="file"
          :title="t('workbench.pick_a_file')"
          :detail="t('workbench.pick_a_file_detail')"
        >
          <button type="button" data-action="workbench.files" @click="toggleFiles">{{ t('common.files') }}</button>
          <button type="button" data-weak="" data-action="workbench.launch-terminal" @click="void terminal.open()">{{ t("terminal.open") }}</button>
        </WorkbenchState>

        <WorkbenchTerminals
          v-if="terminalTabs.length > 0"
          :port="port"
          :terminals="liveTerminals"
          :active="active?.kind === 'terminal' ? active.id : ''"
          :visible="visible"
          :scheme="scheme"
          :shells="terminal.shells.value"
          :on-load-shells="terminal.loadShells"
          :on-shell="(kind: string) => void terminal.open(kind)"
          :on-open-page="(url: string) => void terminal.openUrl(url)"
          :on-command="terminal.submitted"
          :on-restart="terminal.restart"
          :on-close="(id: string) => { const tab = terminal.tabs.value.find((x) => x.id === id); if (tab) void closeSurface(tab); }"
          :on-error="onError"
          :asking="terminal.asking.value"
          :on-cancel-ask="terminal.cancelAsk"
          :on-confirm-ask="terminal.confirmAsk"
        />

        <div v-if="active?.kind === 'changes'" class="workbench-changes-page">
          <WorkbenchChanges
            :port="port"
            :workspace-root="root"
            :changes="changes"
            :current="''"
            :on-open="(path: string) => { openFile(path); mode = 'diff'; }"
            :on-changed="() => { readChanges(); void readTree(); }"
          >
            <template #commit-button><component :is="commit.button.value" /></template>
            <template #actions><component :is="push.button.value" /></template>
            <template #commit-card><component :is="commit.card.value" /></template>
            <template #below><component :is="push.card.value" /></template>
          </WorkbenchChanges>
        </div>

        <JobRunView
          v-if="active?.kind === 'job'"
          :key="active.id"
          :port="port"
          :id="active.id"
          :name="active.name"
          :shown="visible"
        />

        <AgentRunView
          v-if="active?.kind === 'agent'"
          :key="active.ref"
          :port="port"
          :run="active"
          :shown="visible"
          :on-show-call="onShowCall"
        />

        <template v-if="active?.kind === 'file'">
          <WorkbenchFileBar
            :path="filePath"
            :mode="mode"
            :on-mode="(next: FileMode) => (mode = next)"
            :readable="readable"
            :previewable="false"
            :document="hasDocument"
            :source="source"
            :can-save="canSave"
            :read-only-in="held?.root ?? buffer.file?.root"
            :on-save="() => void save()"
            :on-more="held ? undefined : moreFromBar"
          />

          <WorkbenchState
            v-if="buffer.missing && mode !== 'document'"
            icon="file"
            :title="t('workbench.file_is_gone')"
            :detail="t('workbench.it_may_have_been_moved')"
            alert
          >
            <button type="button" data-action="workbench.reread" @click="void readBuffer(filePath)">{{ t("workbench.read_again") }}</button>
            <button type="button" data-weak="" data-action="workbench.close" @click="openFiles = openFiles.filter((p) => p !== filePath); selected = ''">
              {{ t("workbench.close_tab") }}
            </button>
          </WorkbenchState>

          <DocumentPreview
            v-else-if="mode === 'document'"
            :key="filePath"
            :port="port"
            :path="filePath"
            :shown="visible"
            :refresh="documentRefresh"
            :view="documentState(documentStates, filePath)"
            :on-open-external="canOpenDocument ? openDocument : undefined"
          />

          <div v-else-if="buffer.loading && !buffer.file" class="workbench-empty">{{ t("common.loading") }}</div>

          <div v-else-if="mode === 'diff'" class="workbench-diff" tabindex="0">
            <UnifiedDiff :diff="buffer.diff" :label="filePath" :truncated="buffer.truncated" :target="filePath" :on-line="() => (mode = 'file')" />
          </div>

          <div v-else-if="mode === 'read' && buffer.file" class="workbench-read" tabindex="0">
            <LazyMarkdown :text="buffer.draft" :local="readRefs" />
          </div>

          <div v-else class="workbench-editor">
            <textarea
              class="workbench-code"
              spellcheck="false"
              :aria-label="t('workbench.file_contents')"
              :readonly="!!held || !!buffer.file?.readOnly"
              :value="buffer.draft"
              data-editor="workbench"
              @input="setDraft(($event.target as HTMLTextAreaElement).value)"
              @keydown="editorKey"
              @keyup="editorKey"
              @click="editorKey"
              @contextmenu="editorMenu"
            />
          </div>

          <div class="workbench-status">
            <span>{{ filePath }}</span>
            <span class="workbench-status-fill" />
            <span v-if="cursor[0] === filePath">{{ t("workbench.line_n", { n: cursor[1] }) }}</span>
            <span v-if="dirtyOf(filePath)" class="workbench-status-dirty">{{ t("workbench.unsaved") }}</span>
          </div>
        </template>
      </main>

      <aside v-if="showFiles" class="workbench-explorer">
        <ExplorerResize />
        <WorkbenchExplorerHead
          :port="port"
          :count="files.length + directories.length"
          :revealable="revealable"
          :hidden="hidden"
          :on-hidden="toggleHidden"
          :on-reveal="revealRoot"
        />
        <label class="workbench-search">
          <StudioIcon name="search" />
          <input
            :aria-label="t('workbench.search_files')"
            data-action="workbench.search"
            :value="query"
            :placeholder="t('workbench.search_files')"
            @input="query = ($event.target as HTMLInputElement).value"
          />
          <button v-if="query" type="button" data-action="workbench.clear-search" :aria-label="t('common.clear_search')" @click="query = ''">
            <StudioIcon name="close" />
          </button>
        </label>
        <p v-if="listFailed" class="workbench-error" role="alert">{{ listFailed }}</p>
        <WorkbenchGit
          :port="port"
          :workspace-root="root"
          :changes="changes"
          :repos="gitRepos.repos.value"
          :failure="gitRepos.failure.value"
          :on-retry="gitRepos.retry"
          :wrote="glance"
          :current="active?.kind === 'file' && mode === 'diff' ? filePath : ''"
          :on-open="(path: string) => { openFile(path); mode = 'diff'; }"
          :on-changed="() => { readChanges(); void readTree(); }"
          :remote="false"
          :on-open-page="changes.length > 0 ? openChanges : undefined"
        />
        <WorkbenchFileTree
          :rows="rows"
          :collapsed="collapsed"
          :workspace-root="root"
          :changes="changes"
          :current="filePath"
          :on-folder="(path: string) => void toggleFolder(path)"
          :on-file="openFile"
          :on-menu="offerMenu"
        />
        <WorkbenchFileMenu
          v-if="menu"
          :key="menu.id"
          :menu="menu"
          :root="root"
          :revealable="revealable"
          :platform="platform"
          :on-reveal="reveal"
          :on-open="(path: string) => void port.openDocument(path).catch((e: unknown) => onError(e))"
          :on-close="dismiss"
        />
      </aside>
    </div>

    <WorkbenchSelectionMenu v-if="textMenu" :menu="textMenu" :on-action="runTextMenu" :on-close="closeTextMenu" />

    <div v-if="asking" ref="askBox" class="workbench-close-ask" role="alertdialog" :aria-label="t('workbench.name_is_not_saved', { name: asking.path.split('/').at(-1) ?? asking.path })">
      <p>{{ t("workbench.name_is_not_saved", { name: asking.path.split("/").at(-1) ?? asking.path }) }}</p>
      <div>
        <button type="button" data-action="workbench.close-save" @click="void saveUnsaved()">{{ t("common.save") }}</button>
        <button type="button" data-action="workbench.close-drop" @click="dropUnsaved">{{ t("workbench.dont_save") }}</button>
        <button type="button" data-action="workbench.close-cancel" @click="asking = null">{{ t("common.cancel") }}</button>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* The panel is laid out by the stylesheets; only what has no rule of its own is
   written here, and it names the classes it is meant for. */
.workbench-code {
  flex: 1; min-height: 0; width: 100%; box-sizing: border-box; padding: 10px 12px;
  border: 0; outline: none; resize: none; background: var(--face-code); color: var(--text);
  font: var(--w-reg) var(--fs-meta)/1.6 var(--mono); tab-size: 2; white-space: pre; overflow: auto;
}
.workbench-code[readonly] { color: var(--muted); }
/* The explorer is a grid column of its own. With it folded away the canvas has
   to take that column, or it is drawn in the second one beside nothing. */
.workbench-body:not([data-files]) .workbench-canvas { grid-column: 1; }
.workbench-changes-page { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.workbench-changes-page :deep(.workbench-changes) { max-height: none; flex: 1; border-bottom: 0; }
</style>
