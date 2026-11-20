<script lang="ts">
import { computed, defineComponent, h, onMounted, onUnmounted, ref, watch, type VNodeChild } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { HubPort, RuntimeView, TreeSession, TreeWorkspace } from "../port/hub";
import { FEEDBACK_RETIRED } from "../feedbackchannel";
import { markSettled } from "../boot/gate";
import Feedback, { type FeedbackTab } from "./Feedback.vue";
import { feedbackEntryTab } from "./feedbackentry.vue";
import { useFeedbackUnread } from "./useFeedbackUnread";
import Gutter, { DOCK, RAIL, dockMax, keepWidth, widthOf } from "./Gutter.vue";
import Nav from "./Nav.vue";
import Pane from "./Pane.vue";
import Chrome from "./Chrome.vue";
import Settings from "./Settings.vue";
import Sidebar from "./Sidebar.vue";
import Onboarding from "./Onboarding.vue";
import Welcome from "./Welcome.vue";
import { firstRunScreen } from "./firstRunScreen";
import StudioIcon from "./StudioIcon.vue";
import PaneTabs from "./PaneTabs.vue";
import ToastStack from "./ToastStack.vue";
import Remotes from "./remote/Remotes.vue";
import RemoteAsk from "./remote/RemoteAsk.vue";
import type { PaneReport } from "./panetypes";
import type { PaneView } from "../state/paneview";
import type { AgentPort } from "../port/port";
import type { RemoteAsk as RemoteAskQuestion } from "../port/remote";
import { useFirstRunRead, useLaunchHealth } from "./launchhealth";
import { onRoomWidth } from "./viewport";
import { useAddWorkspace } from "./addws";
import { useEscape } from "./dismiss";
import { onShowsSessionTabsChange, showsSessionTabs } from "../state/prefs";
import { useEscStop } from "../state/escstop";
import { pressedChord, typingElsewhere } from "./keys";
import { listenAction } from "./listen";
import { usePaint } from "./paint";
import { useUnreadBadge, useViewed } from "./unread";
import { singleFlight } from "./singleflight";
import { useWindowKeys, type Shortcut } from "./windowkeys";
import { useZoomKeys } from "./zoomkeys";
import { useLinkRouting } from "./links";
import { useBranchSession } from "./useBranchSession";
import { useSessionSwitch } from "./sessionswitch";
import { rememberActivePane, savedActivePane } from "./activepane";
import Sky from "./Sky.vue";
import StopHint from "./StopHint.vue";

const NO_REPORT: PaneReport = {
  status: null,
  workspace: null,
  title: "",
  steer: 0,
  run: "idle",
  live: false,
  cost: "",
  contextPercent: null,
  context: null,
  mcp: [],
  wallet: "",
  view: "flow",
  rows: 0,
  surfaces: 0,
};

interface Props {
  hub: HubPort;
}

export default defineComponent((props: Props) => {
  const runtimes = ref<RuntimeView[]>([]);
  const tree = ref<TreeWorkspace[]>([]);
  const active = ref(savedActivePane());
  const sw = useSessionSwitch();
  const flights = new Map<string, ReturnType<HubPort["open"]>>();
  const error = ref("");
  const busy = ref(true);
  const rail = ref(true);
  const railW = ref(widthOf(RAIL));
  // The column a docked browser occupies. A remembered width like the rail's,
  // not zero: the docked grid reads it as `--dock-open`, and a pane docked at
  // zero shows a browser nobody can see.
  const dockW = ref(widthOf(DOCK));
  const dockMaxW = ref(dockMax(document.body.clientWidth));
  // What is drawn and what is remembered part company at the limit, the way the
  // touched width and the remembered one do in Gutter: a narrow window clamps
  // the column without forgetting what the reader chose.
  const dockOpen = computed(() => Math.min(dockW.value, dockMaxW.value));
  const report = ref<PaneReport>(NO_REPORT);
  // Every pane's running state, not just the focused one: the rail gates a
  // conversation's pause/close/archive on its own runtime, whichever pane holds it.
  const runs = ref<Record<string, { live: boolean }>>({});
  const feedback = ref<FeedbackTab | null>(null);
  const settingsOpen = ref(false);
  // The machines panel, a sheet like settings rather than a column: the host
  // book is edited rarely, and a panel left open beside the work would be one
  // more thing on screen that never goes away.
  const remotesOpen = ref(false);
  // The question a first connect stopped on. It belongs to the window rather
  // than to a pane — the dial that raised it has no pane yet — and a question
  // nothing renders leaves that dial waiting until the kernel times it out.
  const remoteAsk = ref<RemoteAskQuestion | null>(null);
  /** The section:anchor the sheet opens on, when something named one. */
  const settingsAt = ref<string | undefined>(undefined);
  const sessionTabs = ref(showsSessionTabs());
  const machinePort = computed(() => props.hub.machinePort());
  const machineRt = computed(() => runtimes.value.find((rt) => !rt.host) ?? null);
  const feedbackUnread = ref(0);
  const adder = useAddWorkspace(props.hub, loadTree, (e) => { error.value = reason(e); });

  // The guide runs before the window has a pane, so it is handed the window's
  // own port rather than a pane's: the welcome flag and the model-service read
  // belong to this machine, and there is no session yet to ask.
  const guidePort = ref<AgentPort | null>(null);
  // The welcome scene asked to open the guide; the person put the guide off for
  // this launch. Both are memory-only: the next start asks the machine again.
  const forceGuide = ref(false);
  const deferred = ref(false);
  const welcomeSeen = ref<boolean | undefined>(undefined);
  const setup = ref<boolean | undefined>(undefined);
  const firstSettled = computed(() => welcomeSeen.value !== undefined && setup.value !== undefined);
  const screen = computed(() => firstRunScreen({
    settled: firstSettled.value,
    deferred: deferred.value,
    setupRequired: setup.value === true,
    forceGuide: forceGuide.value,
    welcomed: welcomeSeen.value === true,
  }));
  // Pulses are counters, not flags: two presses of the same key must land twice.
  // The bar belongs to a pane, so this only asks; the focused pane answers it.
  const findPulse = ref(0);
  const findTogglePulse = ref(0);
  // Which panes the person has taken the browser over in. Per pane, because
  // docking one session's browser says nothing about another's.
  const manual = ref<Record<string, boolean>>({});
  // Per pane: asking one conversation for a view says nothing about another's.
  const viewRequests = ref<Record<string, { to: PaneView; n: number }>>({});
  // The two column chords, counted the way the dock's per-pane flag is: one
  // conversation's open files say nothing about another's.
  const filesPulse = ref(0);
  const terminalPulse = ref(0);
  const stopArmed = ref(false);
  // The person's own answer to "does Escape stop at once, or ask once more".
  const escStop = useEscStop();

  const ports = computed(() => new Map(runtimes.value.map((rt) => [rt.id, props.hub.portFor(rt)])));
  const activeRuntime = computed(() => runtimes.value.find((rt) => rt.id === active.value) ?? null);
  const activePort = computed(() => (active.value ? ports.value.get(active.value) ?? null : null));

  const activeTitle = computed(() => {
    const rt = activeRuntime.value;
    if (!rt) return "";
    const row = sessions.value.find((s) => s.path === rt.sessionPath);
    return row?.title || row?.name || rt.name || t("common.new_session");
  });

  const sessions = computed(() => {
    const rows: TreeSession[] = [];
    for (const ws of tree.value) for (const s of ws.sessions) rows.push(s);
    return rows;
  });

  const fail = (e: unknown) => {
    error.value = reason(e);
  };

  // Paint is the only writer of documentElement's dataset: theme, contrast,
  // weight and the active pack are one concern, and a second writer here is
  // what left the appearance settings looking dead.
  const running = computed(() => report.value.run === "running");
  const { theme, setTheme, scheme, contrast, setContrast, weight, setWeight, effects, look, onLook, pack, reloadThemes } =
    usePaint(props.hub, runtimes, running, fail);

  // The pack's sky is drawn, not painted by the stylesheet, so it is a component
  // rather than a token: theme.ts writes --cloud-hi/--ray and sets data-sky, and
  // this is what those declarations are for. Reduced effects drop it entirely,
  // which is the same answer the stylesheet gives every other motion.
  const sky = computed(() => !!pack.value?.sky && effects.value !== "reduced");

  async function loadTree(): Promise<void> {
    tree.value = await props.hub.tree().catch(() => []);
  }

  /** Everything that opens the sheet goes through here, so a caller that named
   *  a setting lands on it instead of on the page it happened to be on. */
  function openSettings(at?: string) {
    settingsAt.value = at;
    settingsOpen.value = true;
  }

  function closeSettings() {
    settingsOpen.value = false;
    settingsAt.value = undefined;
  }

  function openRemotes() {
    remotesOpen.value = true;
  }

  function closeRemotes() {
    remotesOpen.value = false;
  }

  /** Session mutations the column can ask for. Each one refreshes the tree: the
   *  kernel owns the file names, so the row it returns is the only row worth
   *  drawing afterwards. */
  async function renameSession(path: string, title: string): Promise<void> {
    try {
      await props.hub.renameSession(path, title);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function archiveSession(path: string): Promise<void> {
    try {
      await props.hub.archiveSession(path, true);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function deleteSession(path: string): Promise<void> {
    try {
      await props.hub.removeSession(path);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  /** The strip's own order. The kernel keeps a list of open panes, not an
   *  order, so a drag within the window is a display choice and lives here. */
  const paneOrder = ref<string[]>([]);
  const ordered = computed(() => {
    const rank = new Map(paneOrder.value.map((id, at) => [id, at]));
    return [...runtimes.value].sort((a, b) => (rank.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (rank.get(b.id) ?? Number.MAX_SAFE_INTEGER));
  });

  function reorderPane(from: number, to: number): void {
    const ids = ordered.value.map((rt) => rt.id);
    const [moved] = ids.splice(from, 1);
    if (!moved) return;
    ids.splice(to, 0, moved);
    paneOrder.value = ids;
  }

  /** Closing a pane is the kernel's answer to keep, so the list is re-read
   *  rather than patched: the pane this window thinks it closed may be the one
   *  another window has already taken. */
  async function closePane(id: string): Promise<void> {
    try {
      await props.hub.close(id);
      delete runs.value[id];
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function archiveWorkspace(root: string): Promise<void> {
    try {
      await props.hub.removeWorkspace(root);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  /** The guide is over. It may have opened a project, which is a runtime this
   *  window did not have before, so the list is re-read rather than trusted. */
  async function finishGuide(result: { projectRoot?: string }): Promise<void> {
    forceGuide.value = false;
    deferred.value = false;
    welcomeSeen.value = true;
    await refresh();
    if (result.projectRoot && runtimes.value.length === 0) {
      await startConversation();
    }
  }

  const readAll = async (): Promise<void> => {
    try {
      const list = await props.hub.runtimes();
      await loadTree();
      runtimes.value = list;
      if (!list.some((rt) => rt.id === active.value)) active.value = list[0]?.id ?? "";
      busy.value = false;
      markSettled();
    } catch (e) {
      busy.value = false;
      markSettled();
      fail(e);
    }
  };

  // Mount, the five-second poll and onTreeChanged all land here; without the
  // merge they overlapped and the slower answer overwrote the newer tree.
  const refresh = singleFlight(readAll);

  async function startConversation(root?: string): Promise<void> {
    busy.value = true;
    try {
      const rt = await props.hub.open(root ? { root } : {});
      await refresh();
      active.value = rt.id;
    } catch (e) {
      fail(e);
    } finally {
      busy.value = false;
    }
  }

  async function openSession(path: string): Promise<void> {
    const held = runtimes.value.find((rt) => rt.sessionPath === path);
    if (held) {
      active.value = held.id;
      // Re-clicking an open conversation returns its pane to the reading view,
      // the way opening it fresh lands there.
      requestView(held.id, "flow");
      return;
    }
    await openPane({ sessionPath: path });
  }

  /** Opening a pane by workspace and session file. A branch lands here with the
   *  root it was cut from, so the new conversation opens beside its source. */
  async function openPane(target: { root?: string; sessionPath: string }): Promise<void> {
    const wanted = target.sessionPath;
    if (wanted && sw.opening(wanted)) return;
    const ticket = sw.begin(wanted);
    busy.value = true;
    try {
      let flight = flights.get(wanted);
      if (!flight) {
        flight = props.hub.open(target);
        flights.set(wanted, flight);
        const done = () => { if (flights.get(wanted) === flight) flights.delete(wanted); };
        void flight.then(done, done);
      }
      const rt = await flight;
      if (sw.current(ticket)) {
        active.value = rt.id;
        sw.landed(ticket);
      }
      await refresh();
    } catch (e) {
      fail(e);
    } finally {
      sw.end(ticket);
      busy.value = false;
    }
  }

  const branchSession = useBranchSession({ panePorts: ports, runtimes, runsRef: runs, openPane });

  /** A pane on another machine. Its first run may install something over there,
   *  so the panel stays up until the pane exists: a refusal then lands on the
   *  sheet that asked for it rather than on the window behind it. */
  async function openRemote(req: { host: string; workspace?: string; sessionPath?: string }): Promise<void> {
    busy.value = true;
    try {
      const rt = await props.hub.openRemote(req);
      await refresh();
      active.value = rt.id;
      remotesOpen.value = false;
    } catch (e) {
      fail(e);
    } finally {
      busy.value = false;
    }
  }

  /** Answering is the only thing that unblocks a dial, so the dialog is emptied
   *  first: a second press must not try to answer a question the kernel has
   *  already settled.
   *
   *  `remember` is accepted and not forwarded. The port carries a consent and a
   *  secret, and the far side writes a fingerprint on any acceptance it is
   *  given — forwarding the box would promise a choice this call cannot express.
   */
  function answerRemote(accept: boolean, _remember: boolean, text?: string): void {
    const ask = remoteAsk.value;
    if (!ask) return;
    remoteAsk.value = null;
    props.hub.answerRemote(ask.askId, accept, text ?? "");
  }

  function onReport(id: string, next: PaneReport) {
    runs.value = { ...runs.value, [id]: { live: next.live } };
    if (id === active.value) report.value = next;
  }

  // A toggle flips the selected preference between the two explicit schemes.
  // Choosing light while the OS is light must still be a light window, so it
  // reads the resolved scheme rather than the preference.
  function toggleTheme() {
    setTheme(scheme.value === "dark" ? "light" : "dark");
  }

  /** Asking for a view is a counter, not a value: the pane's own state answers
   *  it, so the second press of the same chord has to be a second request. The
   *  request is the pane's own — bumping one leaves every other pane's reference
   *  unchanged, so only the pane asked for switches. */
  function requestView(id: string, to: PaneView) {
    viewRequests.value = { ...viewRequests.value, [id]: { to, n: (viewRequests.value[id]?.n ?? 0) + 1 } };
  }
  function showView(to: PaneView) {
    if (active.value) requestView(active.value, to);
  }

  // The files bar belongs to a pane, so no active session is an error the person
  // sees, not a silent no-op; an open sheet closes before the pane answers.
  function openFiles() {
    if (!activePort.value) {
      error.value = t("app.open_a_session_first_to");
      return;
    }
    closeSettings();
    filesPulse.value += 1;
  }

  // Escape below a sheet and Escape over a running turn are different intents,
  // and only one of them may act: while anything is up the sheet takes the press.
  const sheetUp = computed(() => settingsOpen.value || remotesOpen.value || !!feedback.value);
  const docked = computed(() => !!active.value && !!manual.value[active.value]);
  const closeBrowser = () => {
    if (active.value) manual.value = { ...manual.value, [active.value]: false };
  };
  const stopTurn = () => {
    const p = activePort.value;
    if (p) void p.cancel().catch(fail);
  };

  useLinkRouting(activePort, () => showView("browser"), fail);

  // The window's shortcuts, named by the action each one performs — the same
  // identity the control on screen carries, because they are the same thing
  // asked for two ways.
  const zoomKeys = useZoomKeys(look, onLook);
  const shortcuts = computed<Shortcut[]>(() => [
    { chord: "b", fields: true, action: "chrome.rail", run: () => { rail.value = !rail.value; } },
    { chord: ",", fields: true, action: "chrome.settings", run: () => openSettings() },
    { chord: "f", action: "transcript.find", run: () => { findPulse.value += 1; } },
    { chord: "1", fields: true, action: "pane.view", run: () => showView("flow") },
    { chord: "2", fields: true, action: "pane.view", run: () => showView("analysis") },
    { chord: "3", fields: true, action: "pane.view", run: () => showView("browser") },
    ...zoomKeys.value,
  ]);
  useWindowKeys(shortcuts, {
    browser: docked,
    settings: sheetUp,
    running,
    confirm: computed(() => escStop.value === "confirm"),
    closeBrowser,
    stop: stopTurn,
    onArm: (armed: boolean) => { stopArmed.value = armed; },
  });

  // Viewing is the window's decision: the pane in front is seen when it becomes
  // the one in front, when a turn ends in it, and whenever it is unread while
  // the window has focus. While a sheet covers it, nothing counts as seen.
  const viewed = useViewed({
    runtimes,
    ports,
    active: computed(() => (settingsOpen.value || feedback.value || remotesOpen.value ? "" : active.value)),
    tree,
    remote: {},
    reload: refresh,
  });
  useUnreadBadge(viewed.tree, viewed.remote);

  const onRailW = (w: number) => {
    railW.value = w;
    keepWidth(RAIL, w);
  };

  let stopRoom: (() => void) | undefined;
  let stopTree: (() => void) | undefined;
  let stopTabs: (() => void) | undefined;
  let stopAsk: (() => void) | undefined;
  let stopTerminal: (() => void) | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  // The guide is a first-run question this machine answers, so it is asked once
  // at mount rather than on every tree change. A machine that cannot answer is
  // treated as one that has been here before: the sequence must never be what
  // stands between someone and their session.
  useFirstRunRead(machinePort, (v) => { setup.value = !!v?.required; }, (seen) => { welcomeSeen.value = seen; });
  useLaunchHealth(machinePort, () => (setup.value ? { required: true } : null), welcomeSeen);
  useFeedbackUnread(
    computed(() => (!FEEDBACK_RETIRED && machineRt.value ? machinePort.value : null)),
    computed(() => machineRt.value?.id ?? ""),
    feedbackUnread,
    (n) => { feedbackUnread.value = n; },
    computed(() => feedback.value !== null),
  );

  // Escape closes the panel, and stands aside while a link-layer question is up:
  // that one answers a blocked dial, and one press must not settle both.
  useEscape(computed(() => remotesOpen.value && !remoteAsk.value), closeRemotes);

  onMounted(() => {
    void refresh();
    // The pack behind the window is read once here: paint's own watcher only
    // re-reads it when a local runtime appears, which is after this.
    reloadThemes();
    guidePort.value = machinePort.value;
    // The docked column may take at most half the room it is drawn in, and the
    // room moves with the window.
    stopRoom = onRoomWidth((width) => { dockMaxW.value = dockMax(width); });
    stopTree = props.hub.onTreeChanged(() => { void refresh(); });
    stopTabs = onShowsSessionTabsChange(() => { sessionTabs.value = showsSessionTabs(); });
    // The one subscription a first connect depends on: a question nobody renders
    // is a dial nobody can release.
    stopAsk = props.hub.onRemoteAsk((ask) => { remoteAsk.value = ask; });
    // The keys sheet already promises this chord, and the workbench's terminal
    // page is what it opens: without a listener the page had no entry point.
    stopTerminal = listenAction(window, "keydown", {
      action: "workbench.launch-terminal",
      listener: (e) => {
        const ev = e as KeyboardEvent;
        if (ev.defaultPrevented || ev.isComposing || ev.repeat) return;
        if (!active.value || typingElsewhere(ev.target) || !pressedChord(ev, "`")) return;
        ev.preventDefault();
        terminalPulse.value += 1;
      },
    });
    timer = setInterval(() => { void refresh(); }, 5000);
  });
  onUnmounted(() => {
    stopRoom?.();
    stopTree?.();
    stopTabs?.();
    stopAsk?.();
    stopTerminal?.();
    if (timer) clearInterval(timer);
  });

  watch(active, (id) => { report.value = NO_REPORT; rememberActivePane(id); });

  const sideless = (content: VNodeChild[]) => h("div", { class: "cols" }, content);

  return () => {
    const ask = remoteAsk.value;
    return h("div", {
      class: "app",
      "data-run": report.value.run,
      "data-rail": rail.value ? "on" : "off",
      "data-nav": "on",
      "data-browser": report.value.view === "browser" ? "on" : "off",
      "data-plan": report.value.status?.plan ? "on" : "off",
      "data-apv": report.value.status?.toolApprovalMode ?? "ask",
      "data-prefs": settingsOpen.value ? "" : undefined,
      "data-tabs": runtimes.value.length > 1 ? "" : undefined,
      style: { "--rail-open": `${railW.value}px`, "--dock-open": `${dockOpen.value}px` },
    }, [
      h(Chrome, {
        port: activePort.value,
        hub: props.hub,
        hasHub: true,
        status: report.value.status,
        workspace: report.value.workspace,
        title: activeTitle.value,
        steer: report.value.steer,
        theme: scheme.value,
        view: report.value.view,
        rows: report.value.rows,
        surfaces: report.value.surfaces,
        onTheme: toggleTheme,
        onFind: () => { findPulse.value += 1; },
        onView: showView,
        onError: fail,
        onOpenProject: () => adder.add(),
      }),
      // Behind everything: the active pack's own sky. It draws nothing unless a
      // pack asked for one, so a window on the default palette is unchanged.
      sky.value ? h(Sky) : null,
      sideless([
        h(Nav, {
          at: settingsOpen.value ? "" : null,
          onGo: () => openSettings(),
          onFiles: openFiles,
          onHome: () => closeSettings(),
          onRail: () => { rail.value = !rail.value; },
          onFeedback: () => { feedback.value = feedbackEntryTab(feedbackUnread.value); },
          onNew: () => void startConversation().catch(fail),
          feedbackUnread: feedbackUnread.value,
          shown: true,
        }),
        h("aside", { class: "rail", hidden: !rail.value, "aria-label": t("app.workspaces") }, [
          h(Sidebar, {
            hub: props.hub,
            tree: viewed.tree.value,
            active: active.value,
            activeSessionPath: activeRuntime.value?.sessionPath,
            busy: busy.value,
            opening: sw.path,
            openingPending: sw.pending,
            onOpenSession: (path: string) => void openSession(path),
            onNewSession: (root?: string) => void startConversation(root).catch(fail),
            onAddWorkspace: () => adder.add(),
            onArchiveWorkspace: (root: string) => void archiveWorkspace(root),
            onRenameSession: (path: string, title: string) => void renameSession(path, title),
            onArchiveSession: (path: string) => void archiveSession(path),
            onDeleteSession: (path: string) => void deleteSession(path),
            runs: runs.value,
            onPauseSession: (runtimeId: string) => { const p = ports.value.get(runtimeId); if (p) void p.cancel().catch(fail); },
            onCloseSession: (runtimeId: string) => void closePane(runtimeId),
            onBranchSession: branchSession,
            reload: refresh,
            onOpenSettings: openSettings,
            onOpenRemotes: openRemotes,
            onError: fail,
          }),
        ]),
        h(Gutter, {
          edge: "l",
          span: RAIL,
          width: railW.value,
          label: t("app.resize_the_workspace_rail"),
          open: rail.value,
          onWidth: onRailW,
          onOpen: (on: boolean) => { rail.value = on; },
        }),
        h("div", { class: "main" }, [
          // The strip is a display choice the component itself can refuse (see
          // state/prefs), so mounting it here never adds a second way to open or
          // close a session that the kernel does not already have.
          h(PaneTabs, {
            runtimes: ordered.value,
            active: active.value,
            onSelect: (id: string) => { sw.cancel(); active.value = id; },
            onClose: (id: string) => void closePane(id),
            onNew: () => void startConversation().catch(fail),
            onReorder: reorderPane,
          }),
          h("div", { class: "panes", "aria-busy": busy.value || sw.pending || undefined }, runtimes.value.length === 0
            ? [h("div", { class: "panes-empty" }, [
                h("span", { class: "mk", "aria-hidden": "true" }, [h(StudioIcon, { name: "chat" })]),
                h("p", { class: "t" }, t("app.no_session_open")),
                h("p", { class: "h" }, t("app.you_can_start_without_a")),
                h("button", { "data-action": "session.new", type: "button", onClick: () => void startConversation().catch(fail) }, t("common.start_a_new_session")),
                h("button", { "data-action": "workspace.add", type: "button", disabled: adder.busy, onClick: () => adder.add() }, t("app.add_folder")),
              ])]
            : ordered.value.map((rt) => {
                const port = ports.value.get(rt.id);
                if (!port) return null;
                return h(Pane, {
                  key: rt.id,
                  rt,
                  port,
                  hub: props.hub,
                  title: rt.id === active.value ? activeTitle.value : (rt.name || t("common.new_session")),
                  active: rt.id === active.value,
                  sideHost: null,
                  side: false,
                  onFocus: () => { sw.cancel(); active.value = rt.id; },
                  visible: rt.id === active.value,
                  onReport,
                  onSessionChanged: refresh,
                  pulse: 0,
                  viewRequest: viewRequests.value[rt.id],
                  findPulse: findPulse.value,
                  findTogglePulse: findTogglePulse.value,
                  filesPulse: filesPulse.value,
                  terminalPulse: terminalPulse.value,
                  needsProject: false,
                  onOpenProject: () => adder.add(),
                  onKeepHere: () => {},
                  onSettings: (at?: string) => openSettings(at),
                  onTurnDone: viewed.turnDone,
                  theme: scheme.value,
                  dockW: dockOpen.value,
                  dockMax: dockMaxW.value,
                  onDockW: (w: number) => { dockW.value = w; keepWidth(DOCK, w); },
                  manualBrowser: !!manual.value[rt.id],
                  onManualBrowser: (on: boolean) => { manual.value = { ...manual.value, [rt.id]: on }; },
                });
              })),
        ]),
      ]),
      adder.pathOpen ? h("div", { class: "fbk-veil" }, [
        h("form", { class: "fbk", onSubmit: (e: Event) => { e.preventDefault(); const input = (e.target as HTMLFormElement).querySelector("input"); adder.addPath(input?.value ?? ""); } }, [
          h("label", { class: "fbk-hd" }, [h("span", t("app.add_folder"))]),
          h("input", { class: "field", name: "path", placeholder: "/path/to/project", autofocus: true }),
          h("div", { class: "fbk-body" }, [
            h("button", { type: "button", class: "btn", onClick: () => adder.closePath() }, t("common.cancel")),
            h("button", { type: "submit", class: "btn", "data-primary": "" }, t("common.add")),
          ]),
        ]),
      ]) : null,
      error.value ? h("div", { class: "errbar", role: "alert" }, [
        h("span", error.value),
        h("button", { type: "button", "aria-label": t("common.close"), onClick: () => { error.value = ""; } }, h(StudioIcon, { name: "close" })),
      ]) : null,
      FEEDBACK_RETIRED || !feedback.value ? null : h(Feedback, {
        port: machinePort.value,
        tab: feedback.value,
        onClose: () => { feedback.value = null; },
        onError: (message: string) => { error.value = message; },
        onUnread: (n: number) => { feedbackUnread.value = n; },
        unread: feedbackUnread.value,
      }),
      settingsOpen.value ? h(Settings, {
        hub: props.hub,
        port: activePort.value ?? machinePort.value,
        networkPort: machinePort.value,
        localWorkspacePort: activePort.value ?? machinePort.value,
        status: report.value.status,
        theme: theme.value,
        reloadThemes,
        onTheme: setTheme,
        contrast: contrast.value,
        onContrast: setContrast,
        weight: weight.value,
        onWeight: setWeight,
        look: look.value,
        onLook,
        onClose: closeSettings,
        onChanged: () => { void refresh(); },
        at: settingsAt.value,
        networkHost: activeRuntime.value?.host,
        onSessionsRecovered: () => { void refresh(); },
        workspaceRoot: activeRuntime.value?.root ?? undefined,
        onError: fail,
      }) : null,
      // The machines panel, in the feedback sheet's frame because that is the
      // one modal body this window already has. Every way out — its own header
      // button, Escape — closes only this sheet.
      remotesOpen.value ? h("div", { class: "fbk-veil" }, [
        h("div", { class: "fbk", role: "dialog", "aria-modal": "true", "aria-label": t("settings.other_machines") }, [
          h("div", { class: "fbk-body" }, [
            h(Remotes, {
              hub: props.hub,
              onOpenRemote: (req: { host: string; workspace?: string; sessionPath?: string }) => void openRemote(req),
              onError: fail,
              onClose: closeRemotes,
            }),
          ]),
        ]),
      ]) : null,
      // One corner per window, not one per pane: state/toasts is a single
      // queue, and a stack inside each pane would draw the same notice twice.
      h(ToastStack, { onError: fail }),
      // The second half of the Escape chord: the first press arms the stop, and
      // this is the only thing that says so on screen.
      h(StopHint, { armed: stopArmed.value }),
      screen.value === "guide" ? h(Onboarding, {
        hub: props.hub,
        port: guidePort.value ?? machinePort.value,
        onDone: (result: { projectRoot?: string }) => void finishGuide(result),
        onError: fail,
        onOpenSettings: (at?: string) => { deferred.value = true; openSettings(at); },
      }) : null,
      // A machine already configured but never met: the opening scene, not the
      // key guide. Its start opens the guide; its skip marks the welcome seen.
      screen.value === "welcome" ? h(Welcome, {
        seen: welcomeSeen.value === true,
        onStart: () => { forceGuide.value = true; },
        onSkip: () => void finishGuide({}),
      }) : null,
      // Drawn last: a connect is blocked on this answer, so nothing the window
      // can draw may land over the question it is waiting for.
      ask ? h(RemoteAsk, { ask, onDecide: answerRemote }) : null,
    ]);
  };
}, {
  name: "App",
  props: ["hub"],
});
</script>
