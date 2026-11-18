<script lang="ts">
import { computed, defineComponent, h, onMounted, onUnmounted, provide, reactive, ref, watch, watchEffect, type VNodeChild } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { moneyList } from "../i18n/format";
import type { Checkpoint, ContextBreakdown, McpEntry, SessionStatus, WorkspaceChanges, WorkspaceGit } from "../port/port";
import type { HubPort, RuntimeView } from "../port/hub";
import type { TrajectoryRead } from "../port/wire";
import type { ConnectOffer } from "./ConnectNote.vue";
import type { PaneView } from "../state/paneview";
import { initialState, reduce, type SessionEvent } from "../state/session";
import { refreshTodos } from "../state/restore";
import { initialTraj, reduceTraj } from "../state/trajectory";
import { pairCheckpoints } from "../state/checkpoints";
import { useHistoryPages } from "./useHistoryPages";
import { useSubmit } from "./usesubmit";
import { useGateActions } from "./gates";
import { useQueueActions } from "./queueactions";
import { useRewindActions } from "./rewind";
import { useReplyActions } from "./reply";
import { useSurfaceReplay } from "./surface-replay";
import { streamBudget } from "../port/streambudget";
import { useStatusRefresh } from "./useStatusRefresh";
import { useStatusPoll } from "./useStatusPoll";
import { useWorkspaceInfo } from "./useWorkspaceInfo";
import { useCheckpointRefresh } from "./useCheckpointRefresh";
import { useWallet } from "./wallet";
import { hasPendingDecision, runState } from "./decisions";
import { noteFault } from "./fault";
import { draftKey } from "./drafts";
import Composer from "./Composer.vue";
import ErrorBar from "./ErrorBar.vue";
import Queue, { waiting } from "./Queue.vue";
import Transcript from "./Transcript.vue";
import Metrics from "./Metrics.vue";
import { railOf } from "./panels/derive";
import BrowserPanel from "./browser/BrowserPanel.vue";
import WorkbenchPanel from "./workbench/WorkbenchPanel.vue";
import TerminalView from "./terminal/TerminalView.vue";
import Find from "./find/Find.vue";
import { useBrowserFull } from "./browserfull";
import { useBroughtResults } from "./useBroughtResults";
import { useAgentRequest } from "./useAgentRequest";
import { AgentRunOpener } from "./cards/SubagentGroup.vue";
import { JobsLiveContext, type JobsLive } from "./JobRunView.vue";
import { jobName } from "./panels/JobName.vue";
import { cancelJob } from "./jobcancel";
import { useOnBump } from "./useOnBump";
import { useShowView } from "./showview";
import type { PaneReport } from "./panetypes";

export type { PaneReport };

interface PaneProps {
  port: import("../port/port").AgentPort;
  hub?: Pick<HubPort, "openWith" | "openWithTargets">;
  rt: RuntimeView;
  title: string;
  active: boolean;
  sideHost: HTMLElement | null;
  side: boolean;
  onFocus: () => void;
  onReport: (id: string, report: PaneReport) => void;
  visible: boolean;
  onSessionChanged: () => void;
  onTurnDone?: (id: string) => void;
  pulse: number;
  findPulse: number;
  findTogglePulse?: number;
  filesPulse?: number;
  terminalPulse?: number;
  viewRequest?: { to: PaneView; n: number };
  sessionRef?: { n: number; id: string; title: string };
  onSettings: (section?: string) => void;
  onFeedback?: () => void;
  onOpenSession?: (path: string) => void;
  needsProject: boolean;
  onOpenProject: () => void;
  onKeepHere: () => void;
  connect?: ConnectOffer;
  theme: string;
  dockW: number;
  dockMax: number;
  onDockW: (w: number) => void;
  manualBrowser?: boolean;
  onManualBrowser?: (on: boolean) => void;
  alert?: VNodeChild;
  failure?: VNodeChild;
}

type TrajEvent = Parameters<typeof reduceTraj>[1];

const NO_ERROR = "";

/** What fills the workbench column. Only one is drawn at a time; all three stay
 *  mounted, because each holds state a switch must not cost (open files, page
 *  history, a shell's screen). */
type Column = "browser" | "files" | "terminal";

export default defineComponent((props: PaneProps) => {
  const s = ref(initialState);
  const traj = ref(initialTraj);
  const status = ref<SessionStatus | null>(null);
  const ready = ref(false);
  const checkpoints = ref<Checkpoint[]>([]);
  const git = ref<WorkspaceGit | null>(null);
  const tree = ref<WorkspaceChanges | null>(null);
  const flow = ref<HTMLDivElement | null>(null);
  const inserted = ref<{ n: number; text: string; session?: { id: string; title: string } }>({ n: 0, text: "" });

  const dispatch = (ev: SessionEvent) => { s.value = reduce(s.value, ev); };
  const trajDispatch = (ev: TrajEvent) => { traj.value = reduceTraj(traj.value, ev); };

  const fail = (e: unknown, from?: string) => {
    const shown = reason(e);
    noteFault(e, shown);
    dispatch({ kind: "__error", text: shown, from } as SessionEvent);
  };

  const applyStatus = (next: SessionStatus) => {
    if (next.running !== undefined) dispatch({ kind: "__running", running: next.running } as SessionEvent);
    status.value = next;
    ready.value = true;
  };

  const refreshStatus = useStatusRefresh(() => props.port, applyStatus, () => {});
  const surfaceReplay = useSurfaceReplay(() => props.port, dispatch);
  const { load: readHistory, paging: historyPaging } = useHistoryPages(() => props.port, dispatch);
  const root = computed(() => status.value?.workspaceRoot || status.value?.cwd || props.rt.root);
  const workspace = useWorkspaceInfo(() => props.port, () => root.value, ready);
  const [walletState, refreshWallet] = useWallet(() => props.port, () => status.value?.modelRef);

  const running = computed(() => s.value.running || !!status.value?.running);
  const blocked = computed(() => hasPendingDecision(status.value));
  const run = computed(() => runState({ blocked: blocked.value, running: running.value, hasItems: s.value.items.length > 0, terminal: s.value.terminal }));
  const paired = computed(() => pairCheckpoints(s.value.items, checkpoints.value));
  // The rail's walk is the pane's: it reads the items and executions this pane
  // already holds, and Metrics draws it rather than deriving a second copy.
  const rail = computed(() => railOf(s.value.items, s.value.executions, s.value.subagentPhase));
  const yolo = computed(() => status.value?.toolApprovalMode === "yolo");
  const readChanges = (current: () => boolean = () => true) => {
    void props.port.changes().then((c) => { if (current()) tree.value = c; }).catch(() => { if (current()) tree.value = null; });
  };

  const replayTrajectory = (read: TrajectoryRead) => {
    trajDispatch({ kind: "__coverage", availability: read.availability });
    read.events.forEach((ev) => trajDispatch(ev));
  };

  const rebuild = () => {
    void surfaceReplay.refresh();
    void readHistory().catch(() => {});
  };

  const reloadSession = (current: () => boolean = () => true): Promise<void> => {
    void surfaceReplay.refresh(current);
    traj.value = initialTraj;
    void props.port.trajectory().then((r) => { if (current()) replayTrajectory(r); }).catch(() => {});
    void props.port.checkpoints().then((cps) => { if (current()) checkpoints.value = cps; }).catch(() => { if (current()) checkpoints.value = []; });
    readChanges(current);
    const history = readHistory(current);
    void props.port.status().then((st) => { if (current()) applyStatus(st); }).catch((e) => { if (current()) fail(e, "status"); });
    void refreshWallet();
    props.onSessionChanged();
    return history.then(() => {});
  };

  const submit = useSubmit({
    port: () => props.port,
    running: () => running.value,
    dispatch: dispatch as (ev: never) => void,
    trajDispatch,
    refreshStatus,
    fail,
  });

  const queueActions = useQueueActions({
    port: () => props.port,
    dispatch,
    fail,
    moved: () => s.value.queueMoved,
    sessionState: () => (ready.value ? "settled" : "pending"),
    sessionPath: () => status.value?.sessionPath ?? props.rt.sessionPath ?? "",
  });

  const rewind = useRewindActions(
    () => props.port,
    reloadSession,
    queueActions.onRestoreText,
    () => JSON.stringify([props.rt.id, props.rt.root, status.value?.sessionPath ?? props.rt.sessionPath]),
    () => running.value,
  );

  const gate = useGateActions({ port: props.port, dispatch, refreshStatus, fail, onRevise: () => {} });
  const reply = useReplyActions({
    port: props.port,
    items: () => s.value.items,
    checkpoints: () => checkpoints.value,
    running: () => running.value,
    model: () => status.value?.label,
    submit: (line) => submit(line).then((r) => r === true),
    reloadSession: () => reloadSession(),
    onSettings: props.onSettings,
    onRunDetail: () => {},
    onError: fail,
    branchName: () => `${props.title} · ${t("common.branch")}`,
    onBranched: props.onOpenSession,
  });

  // A reader asking to see a call's result again at the end of the transcript.
  // A view over the calls the transcript already holds, so a call rewound away
  // leaves the list by itself.
  const brought = useBroughtResults(() => s.value.items);

  // Every route into a view goes through one place: the chrome asking for the
  // workbench and the globe taking the browser over are the same act, so the
  // pane's own view and the dock's takeover flag are decided together. A pane
  // that is not docked shows the workbench at full width instead.
  const [view, showView] = useShowView(props.rt.id, (on) => props.onManualBrowser?.(on));

  // A browser column docked beside the conversation is the pane's own takeover
  // flag plus the flow view; the workbench view is the same column at full
  // width. Both draw the same column, so which one is on is what `visible` and
  // the report's own `view` say.
  const docked = computed(() => !!props.manualBrowser && view.value === "flow");
  const workbench = computed(() => docked.value || view.value === "browser");
  const browserSize = useBrowserFull(docked, () => props.active && props.visible, blocked);

  // The column is the pane's own choice, not a preference: it lives as long as
  // the pane instance, so a pane remounted for another conversation starts on
  // the pages again.
  const column = ref<Column>("browser");
  const landAt = ref<{ item: string; n: number } | { call: string; n: number } | null>(null);
  const openColumn = (to: Column) => {
    column.value = to;
    // A closed column draws nothing, so asking for a page has to open it.
    if (!workbench.value) showView("browser");
  };
  // The panel puts itself away; what that means is the column's, and the column
  // belongs to the view it is drawn in.
  const closeColumn = () => {
    column.value = "browser";
    if (docked.value) props.onManualBrowser?.(false);
    else if (view.value === "browser") showView("flow");
  };
  // Hiding is this element's own inline style: every one of the three declares
  // `display` in a stylesheet, and a `[hidden]` attribute loses to it.
  const held = (which: Column) => (column.value === which ? undefined : { display: "none" });

  const agentRuns = useAgentRequest(() => props.port, () => openColumn("files"));
  provide(AgentRunOpener, agentRuns.open);

  const jobsLive = reactive<JobsLive>({ jobs: [] });
  watchEffect(() => {
    jobsLive.jobs = [...(status.value?.jobs ?? []), ...(status.value?.endedJobs ?? [])];
    jobsLive.cancel = (id) => cancelJob(props.port, id, (e) => fail(e, "jobs"), refreshStatus).then(() => undefined);
    jobsLive.open = (job) => agentRuns.openJob({ id: job.id, name: jobName(job) });
  });
  provide(JobsLiveContext, jobsLive);

  // Find is the pane's own bar over its own scroller: the chord opens it on the
  // pane that has focus, never on every open session at once.
  const findOpen = ref(false);
  // The scroller the bar searches — the transcript's own box, not the body it
  // sits in: the bar reads that element's rows and scrolls it to a hit, and the
  // body is neither. Taken from the mounted subtree rather than inside the
  // render function, because reading a template ref there makes the write to it
  // re-trigger the render that owns it.
  const findHost = ref<HTMLElement | null>(null);

  // A request from the chrome to show another view, counted so asking for the
  // same view twice still lands; the pane's own state is what answers.
  watch(() => props.viewRequest, (req) => {
    if (req) showView(req.to);
  });
  // Pulses are counters, not flags: the same chord pressed twice must land
  // twice, and only the focused pane answers it.
  watch(() => props.findPulse, (beat) => { if (beat && props.active) findOpen.value = true; });
  watch(() => props.findTogglePulse, (beat) => { if (beat && props.active) findOpen.value = !findOpen.value; });
  // The two column chords name a page rather than the column: one opened onto
  // another's page is still this one's.
  watch(() => props.filesPulse, (beat) => { if (beat && props.active) openColumn("files"); });
  watch(() => props.terminalPulse, (beat) => { if (beat && props.active) openColumn("terminal"); });
  const contextRead = ref<ContextBreakdown | null>(null);
  const mcpRead = ref<McpEntry[]>([]);

  const report = computed<PaneReport>(() => ({
    status: status.value,
    workspace: workspace.value,
    title: props.title,
    steer: waiting(queueActions.queue.value).filter((it) => it.intent === "steer").length,
    run: run.value,
    live: running.value || blocked.value,
    cost: s.value.metrics.coverage === "none" ? "" : moneyList(s.value.metrics.cost),
    contextPercent: status.value && status.value.window > 0 ? Math.min(100, Math.round((status.value.used / status.value.window) * 100)) : null,
    context: contextRead.value,
    mcp: mcpRead.value,
    wallet: walletState.value.kind === "read" ? walletState.value.reading.display : "",
    interactions: s.value.interactions,
    // A docked column is the workbench view drawn beside the conversation, and
    // the chrome reads this one field to say so (`data-browser` is what the
    // docked grid turns on).
    view: docked.value ? "browser" : view.value,
    rows: traj.value.rows.length,
    surfaces: s.value.panels.length + s.value.views.length,
  }));

  let sub: (() => void) | undefined;
  let releaseStream: (() => void) | undefined;

  onMounted(() => {
    // `flow` is the transcript's own scroller: the transcript takes the same
    // ref for its root, so after mount it is either that element or the body
    // that holds it. Both spellings are read rather than guessed at.
    const body = flow.value;
    findHost.value = body ? (body.matches('[data-pane="flow"]') ? body : body.querySelector<HTMLElement>('[data-pane="flow"]')) : null;
    void readHistory().catch((e) => fail(e, "history"));
    void props.port.trajectory().then(replayTrajectory).catch(() => {});
    void props.port.status().then(applyStatus).catch((e) => { if (!ready.value) { ready.value = true; fail(e, "status"); } });
    void props.port.checkpoints().then((cps) => { checkpoints.value = cps; }).catch(() => { checkpoints.value = []; });
    void props.port.workspaceGit().then((g) => { git.value = g; }).catch(() => { git.value = null; });
    readChanges();
    void props.port.context().then((c) => { contextRead.value = c; }).catch(() => { contextRead.value = null; });
    void props.port.mcp().then((c) => { mcpRead.value = c.servers; }).catch(() => { mcpRead.value = []; });
    const subscription = props.port.subscribe(
      (ev) => {
        surfaceReplay.observe(ev);
        dispatch(ev);
        trajDispatch(ev);
        // The list advances without a todo_write and a refused one is written
        // only there, so the kernel's own list is what the panel draws.
        if (ev.kind === "todo_progress") void refreshTodos(props.port, dispatch);
        if (ev.kind === "turn_started" || ev.kind === "turn_done" || ev.kind === "approval_request" || ev.kind === "ask_request") refreshStatus();
        else if ("decisionReceipt" in ev && ev.decisionReceipt) refreshStatus();
        if (ev.kind === "turn_done") { props.onTurnDone?.(props.rt.id); readChanges(); }
      },
      () => { rebuild(); refreshStatus(); },
    );
    sub = subscription;
    releaseStream = streamBudget.attach(props.rt.id, subscription, refreshStatus, { visible: props.visible, busy: running.value });
    props.onSessionChanged();
  });

  onUnmounted(() => {
    releaseStream?.();
    sub?.();
  });

  watch([() => props.visible, running], ([visible, busy]) => {
    streamBudget.update(props.rt.id, { visible, busy });
  });
  watch(() => props.pulse, (pulse) => {
    if (!pulse) return;
    refreshStatus();
    void surfaceReplay.refresh();
  });
  // A turn ending is when the host list moves with no event of its own, and a
  // mount is the first read of it: both ask the kernel rather than re-deriving.
  watch(running, (busy) => { if (!busy) void refreshTodos(props.port, dispatch); }, { immediate: true });
  useCheckpointRefresh(() => props.port, () => status.value?.sessionPath, () => running.value, (cps) => { checkpoints.value = cps; });
  useStatusPoll(() => running.value && props.visible, refreshStatus);

  watch(report, (next) => props.onReport(props.rt.id, next), { immediate: true, flush: "post" });

  const onQuote = (text: string) => { inserted.value = { n: inserted.value.n + 1, text }; };
  // A conversation the rail asked this pane's composer to hold as a reference.
  // The counter is what makes the same one asked for twice land twice.
  useOnBump(() => props.sessionRef?.n ?? 0, () => {
    const ref = props.sessionRef;
    if (ref) inserted.value = { n: inserted.value.n + 1, text: "", session: { id: ref.id, title: ref.title } };
  });
  const draft = computed(() => draftKey(props.rt.host ?? "", props.rt.root, props.rt.sessionPath || status.value?.sessionPath || ""));
  const queueStrip = computed(() => h(Queue, {
    queue: queueActions.queue.value,
    running: running.value,
    shown: new Set<string>(),
    onSendNow: queueActions.onQueueSendNow,
    onToSteer: queueActions.onQueueToSteer,
    onCancel: queueActions.onQueueCancel,
    onRetry: queueActions.onQueueRetry,
    onPause: queueActions.onQueuePause,
  }));

  return () => h("section", {
    class: "pane",
    "data-pane": "flow",
    "data-browser-size": browserSize.size.value,
    "data-run": run.value,
    "data-plan": s.value.plan.length > 0 || status.value?.goal ? "" : undefined,
    "data-off": props.visible ? undefined : "",
    "aria-hidden": props.visible ? undefined : true,
    "data-active": props.active ? "" : undefined,
    "aria-label": props.title,
    onMousedownCapture: props.active ? undefined : props.onFocus,
    onFocusCapture: props.active ? undefined : props.onFocus,
  }, [
    // The bar floats over the transcript's own scroller, which is why it is a
    // sibling of the body rather than a child of it.
    h(Find, {
      open: findOpen.value,
      onClose: () => { findOpen.value = false; },
      host: findHost.value,
      onError: (e: unknown) => fail(e, "find"),
    }),
    // `data-dock` says the conversation owns the body row, which is what turns
    // the pane into the grid the browser column spans (see dock.css): it is a
    // property of the flow view, not of the person having opened the browser.
    // `data-full` is the other direction — the pages take the whole canvas and
    // the conversation's column is hidden.
    h("div", { class: "pbody", ref: flow, "data-dock": view.value === "flow" ? "" : undefined, "data-full": view.value === "browser" ? "" : undefined }, [
      h("div", { class: "pviews" }, [
        view.value === "analysis" ? h("div", { class: "scroll", "data-pane": "analysis" }, [
          h(Metrics, {
            port: props.port,
            context: contextRead.value,
            mcp: mcpRead.value,
            surfaces: s.value.panels.length + s.value.views.length,
            changes: rail.value.changes,
            tree: tree.value,
            yolo: yolo.value,
            onError: (e: unknown) => fail(e, "metrics"),
            onOpenSettings: props.onSettings,
          }),
        ]) : null,
        h(Transcript, {
          port: props.port,
          historyPaging,
          revision: s.value.revision,
          items: s.value.items,
          brought: { ids: brought.ids.value, bring: brought.bring, dismiss: brought.dismiss },
          entering: s.value.entranceOwed,
          onEntered: (ids: string[]) => dispatch({ kind: "__entered", ids } as SessionEvent),
          waiting: s.value.waiting,
          scroll: flow,
          // Hidden rather than unmounted: a visit to another view must not
          // throw away the transcript's scroll position or its folds. The CSS
          // already hides a `[data-pane="flow"][hidden]` box.
          hidden: view.value !== "flow",
          focus: landAt.value,
          find: null,
          query: "",
          onApprove: gate.onApprove,
          onFullAccess: gate.onFullAccess,
          onPlan: gate.onPlan,
          onAnswer: gate.onAnswer,
          onForget: gate.onForget,
          onSettings: props.onSettings,
          onExtInvoke: gate.onExtInvoke,
          onExtSubmit: gate.onExtSubmit,
          takeovers: s.value.takeovers,
          reply: reply.reply.value,
          onResend: reply.onResend,
          onWithdraw: queueActions.onQueueCancel,
          checkpoints: paired.value,
          onPrepareRewind: rewind.onPrepareRewind,
          onCommitRewind: rewind.onCommitRewind,
          onPrepareFileRevert: rewind.onPrepareFileRevert,
          onCommitFileRevert: rewind.onCommitFileRevert,
          needsProject: props.needsProject,
          onOpenProject: props.onOpenProject,
          onKeepHere: props.onKeepHere,
          onSuggest: onQuote,
          foldWork: false,
          turnRunning: running.value,
          scope: props.rt.sessionPath,
        }),
      ]),
    ]),
    // The browser column is a direct child of the pane so the docked grid can
    // give it a column spanning both the body's row and the composer's; inside
    // the body it would stop at the composer's top edge. The pages, the files
    // and the terminals are the three things it can hold, and all three stay
    // mounted so a switch costs none of their state.
    h("div", { class: "scroll", "data-pane": "browser", hidden: !workbench.value }, [
      h(BrowserPanel, {
        port: props.port,
        visible: props.visible && workbench.value && column.value === "browser",
        // The takeover flag itself, not `docked`: in the workbench view the
        // person's own pages are the column's content just the same, and it is
        // only the docked layout that needs the flow view beside them.
        manual: !!props.manualBrowser,
        onManual: (on: boolean) => props.onManualBrowser?.(on),
        onError: (e: unknown) => fail(e, "browser"),
        full: browserSize.size.value === "full",
        onFull: () => browserSize.toggleFull(),
        style: held("browser"),
      }),
      h(WorkbenchPanel, {
        port: props.port,
        root: root.value,
        onError: (e: unknown) => fail(e, "workbench"),
        // The panel's own close button puts the whole column away, whichever
        // page it was showing.
        onClose: closeColumn,
        visible: props.visible && workbench.value && column.value === "files",
        agentRequest: agentRuns.request.value,
        onShowCall: (call: string) => { landAt.value = { call, n: (landAt.value?.n ?? 0) + 1 }; },
      }),
      h(TerminalView, {
        port: props.port,
        onError: (e: unknown) => fail(e, "terminal"),
        visible: props.visible && workbench.value && column.value === "terminal",
        style: held("terminal"),
      }),
    ]),
    // Kept mounted across views so a half-written prompt survives a visit to
    // analysis; `hidden` takes it out of the layout without its state.
    h("div", { class: "compose", "data-rise": undefined, hidden: view.value !== "flow" }, [
      h(Composer, {
        port: props.port,
        status: status.value,
        running: running.value,
        queued: waiting(queueActions.queue.value).length > 0,
        quote: reply.quote.value,
        restore: queueActions.restored.value,
        inject: inserted.value,
        focus: 0,
        onSubmit: submit,
        onChanged: refreshStatus,
        onError: fail,
        onSettings: props.onSettings,
        side: false,
        queueStrip: queueStrip.value,
        changeCount: 0,
        git: git.value,
        workspace: workspace.value,
        pulse: props.pulse,
        draftKey: draft.value,
      }),
      h("div", { class: "composenotes" }, [
        props.failure ?? null,
        s.value.error ? h(ErrorBar, {
          text: s.value.error,
          onDismiss: () => dispatch({ kind: "__error", text: NO_ERROR } as SessionEvent),
          onReport: props.onFeedback,
        }) : null,
      ]),
    ]),
  ]);
}, {
  name: "Pane",
  props: [
    "port", "hub", "rt", "title", "active", "sideHost", "side", "onFocus", "onReport", "visible",
    "onSessionChanged", "onTurnDone", "pulse", "findPulse", "findTogglePulse", "filesPulse", "terminalPulse",
    "viewRequest", "sessionRef", "onSettings", "onFeedback", "onOpenSession", "needsProject",
    "onOpenProject", "onKeepHere", "connect", "theme", "dockW", "dockMax", "onDockW",
    "manualBrowser", "onManualBrowser", "alert", "failure",
  ],
});
</script>
