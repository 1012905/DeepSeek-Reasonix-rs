<script lang="ts">
import { computed, defineComponent, h, nextTick, onMounted, onUnmounted, ref, watch, type Ref, type VNodeChild } from "vue";
import { t } from "../i18n";
import type { AgentPort, ApprovalVerdict, Checkpoint, RewindPlan, RewindResult, RewindScope } from "../port/port";
import type { ExtensionSurface } from "../port/wire";
import type { PlanAction } from "../port/session";
import type { Item, Waiting } from "../state/session";
import { compactionFold } from "./compactionfold";
import { toolFailed } from "./cards/outcome";
import ApprovalCard from "./cards/ApprovalCard.vue";
import AskCard from "./cards/AskCard.vue";
import CompactionCard from "./cards/CompactionCard.vue";
import ElicitCard from "./cards/ElicitCard.vue";
import ExtensionCard from "./cards/ExtensionCard.vue";
import GuardianCard from "./cards/GuardianCard.vue";
import NoticeCard from "./cards/NoticeCard.vue";
import ReadsCard from "./cards/ReadsCard.vue";
import RememberCard from "./cards/RememberCard.vue";
import SayCard, { type ReplyActions } from "./cards/SayCard.vue";
import ToolCardSafe from "./cards/ToolCardSafe.vue";
import Boundary from "./Boundary.vue";
import Await from "./Await.vue";
import ChartCard from "./chart/ChartCard.vue";
import { chartOfTool } from "./chart/spec";
import UserCard from "./cards/UserCard.vue";
import WorkedLine from "./cards/WorkedLine.vue";
import SubagentGroup, { workLanes } from "./cards/SubagentGroup.vue";
import BroughtResults from "./BroughtResults.vue";
import StudioIcon from "./StudioIcon.vue";
import type { HistoryPaging } from "./useHistoryPages";

interface Props {
  historyPaging?: HistoryPaging;
  items: Item[];
  port?: Pick<AgentPort, "workspaceImageURL"> & Partial<Pick<AgentPort, "surfaceAction" | "surfaceActionState" | "cancelQueued" | "engageQuestion" | "stopToolCall" | "refinePrompt" | "toolOutput">>;
  revision: number;
  waiting: Waiting;
  scroll: Ref<HTMLDivElement | null>;
  hidden: boolean;
  focus: { call: string; n: number } | { item: string; n: number } | null;
  find?: { id: string; nth: number; n: number; mode?: { regex?: boolean; caseSensitive?: boolean } } | null;
  query?: string;
  onApprove: (itemId: string, id: string, v: ApprovalVerdict) => Promise<void>;
  onFullAccess: (itemId: string) => Promise<void>;
  onPlan: (itemId: string, id: string, action: PlanAction) => Promise<void>;
  onAnswer: (itemId: string, id: string, answers: { questionId: string; selected: string[]; value?: Record<string, unknown> }[]) => Promise<void>;
  onForget: (itemId: string, name: string) => void;
  onSettings?: (at?: string) => void;
  onExtInvoke: (name: string) => void;
  takeovers?: Record<string, ExtensionSurface>;
  reply?: ReplyActions;
  onResend?: (turn: number, text: string) => Promise<void>;
  onWithdraw?: (itemId: string) => void | Promise<void>;
  onExtSubmit: (pluginId: string, surfaceId: string, values: Record<string, unknown>) => void | Promise<void>;
  needsProject: boolean;
  onOpenProject: () => void;
  onKeepHere: () => void;
  onSuggest?: (text: string) => void;
  foldWork?: boolean;
  turnRunning?: boolean;
  scope?: string;
  checkpoints: Map<string, Checkpoint>;
  onPrepareRewind: (turn: number, scope: RewindScope) => Promise<RewindPlan>;
  onPrepareFileRevert: (path: string) => Promise<RewindPlan>;
  onCommitFileRevert: (planId: string, resolution?: string) => Promise<RewindResult>;
  onCommitRewind: (planId: string, text?: string) => Promise<RewindResult>;
  entering: string[];
  onEntered: (ids: string[]) => void;
  brought?: { ids: string[]; bring: (id: string) => void; dismiss: (id: string) => void };
}

type Activity = { kind: "activity"; key: string; items: Item[] };
type Plain = { kind: "plain"; key: string; item: Item };
type Agents = { kind: "agents"; key: string; tasks: Extract<Item, { t: "tool" }>[] };
type Group = Activity | Plain | Agents;

const isWork = (item: Item) => (item.t === "tool" && !chartOfTool(item.tool)) || item.t === "reads" || item.t === "worked" || item.t === "guardian";

function grouped(items: Item[]): Group[] {
  const out: Group[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!isWork(item)) {
      out.push({ kind: "plain", key: item.id, item });
      continue;
    }
    const batch: Item[] = [item];
    while (i + 1 < items.length && isWork(items[i + 1])) batch.push(items[++i]);
    // Two or more delegates sent out side by side read as a group of their own,
    // outside the work fold; a lone one stays in the execution group.
    for (const lane of workLanes(batch)) {
      if (lane.agents) out.push({ kind: "agents", key: `agents:${lane.agents[0].id}`, tasks: lane.agents });
      else out.push({ kind: "activity", key: `activity:${lane.items[0].id}`, items: lane.items });
    }
  }
  return out;
}

const failureOf = (item: Item) => item.t === "tool" ? toolFailed(item.tool) : item.t === "reads" ? item.tools.some(toolFailed) : false;

export default defineComponent((props: Props) => {
  const flow = ref<HTMLDivElement | null>(null);
  const unfolded = ref(false);
  const showEarlier = ref<Record<string, boolean>>({});
  const pinned = ref(true);
  const owed = computed(() => new Set(props.entering));

  const fold = computed(() => compactionFold(props.items));
  const morePages = computed(() => props.historyPaging?.hasMore.value ?? false);
  const pageBusy = computed(() => props.historyPaging?.busy.value ?? false);
  const pageFailed = computed(() => props.historyPaging?.failed.value ?? false);
  const shown = computed(() => (unfolded.value ? props.items : props.items.slice(fold.value.cut)));
  const groups = computed(() => grouped(shown.value));

  const nearBottom = () => {
    const root = props.scroll.value;
    if (!root) return true;
    return root.scrollHeight - root.scrollTop - root.clientHeight <= 48;
  };
  const follow = () => {
    const root = props.scroll.value;
    if (!root || props.hidden) return;
    void nextTick(() => { if (pinned.value) root.scrollTop = root.scrollHeight; });
  };
  const onScroll = () => { pinned.value = nearBottom(); };
  const onWheel = (e: WheelEvent) => { if (e.deltaY < 0) pinned.value = false; };

  onMounted(() => {
    const root = props.scroll.value;
    root?.addEventListener("scroll", onScroll, { passive: true });
    root?.addEventListener("wheel", onWheel, { passive: true });
    follow();
    if (props.entering.length) queueMicrotask(() => props.onEntered(props.entering));
  });
  onUnmounted(() => {
    const root = props.scroll.value;
    root?.removeEventListener("scroll", onScroll);
    root?.removeEventListener("wheel", onWheel);
  });
  watch(() => props.revision, follow);
  watch(() => props.hidden, (hidden) => { if (!hidden) follow(); });
  watch(() => props.entering.join("\n"), () => { if (props.entering.length) queueMicrotask(() => props.onEntered(props.entering)); });

  const earlierButton = (group: Activity) => {
    const full = group.items.length > 6;
    if (!full) return null;
    const open = !!showEarlier.value[group.key];
    return h("button", {
      type: "button",
      class: "activity-earlier",
      "data-action": "activity.earlier",
      "aria-expanded": open,
      onClick: () => { showEarlier.value = { ...showEarlier.value, [group.key]: !open }; },
    }, [
      h("span", t("chat.n_earlier_steps", { n: group.items.length - 6 })),
      h("span", { class: "activity-earlier-kinds" }),
      h(StudioIcon, { name: "down" }),
    ]);
  };

  const activity = (group: Activity) => {
    const open = !!showEarlier.value[group.key];
    const rows = open || group.items.length <= 6 ? group.items : group.items.slice(-6);
    const failures = group.items.filter(failureOf).length;
    const running = group.items.some((item) => item.t === "tool" && item.running);
    return h("details", {
      class: "activity-group",
      key: group.key,
      "data-failed": failures > 0 ? "" : undefined,
      "data-running": running ? "" : undefined,
      open: running || failures > 0 || undefined,
    }, [
      h("summary", { class: "activity-head" }, [
        h(StudioIcon, { name: failures > 0 ? "warning" : running ? "loading" : "check", class: "activity-status-icon" }),
        h("span", { class: "activity-title" }, t("chat.work_in_between")),
        h("span", { class: "activity-count" }, t("chat.count_actions", { count: group.items.length })),
      ]),
      h("div", { class: "activity-body" }, [earlierButton(group), ...rows.map((item) => card(item, true))]),
    ]);
  };

  const card = (item: Item, inActivity = false): VNodeChild => {
    switch (item.t) {
      case "user":
        return h(UserCard, {
          item,
          port: props.port as AgentPort | undefined,
          cp: props.checkpoints.get(item.id),
          onResend: props.onResend,
          onWithdraw: props.onWithdraw,
          onPrepareRewind: props.onPrepareRewind,
          onCommitRewind: props.onCommitRewind,
        });
      case "say":
        return item.text || item.reasoning
          ? h(SayCard, { item, settled: item.done, reply: props.reply, quiet: inActivity })
          : null;
      case "tool": {
        if (item.tool.name === "ask") return null;
        const toolCard = h(ToolCardSafe, {
          tool: item.tool,
          running: item.running,
          activity: inActivity,
          children: item.children,
          readOutput: props.port?.toolOutput?.bind(props.port),
          omitted: item.omitted,
          takeover: item.tool.id ? props.takeovers?.[`tool:${item.tool.id}`] : undefined,
          onExtInvoke: props.onExtInvoke,
          onPrepareFileRevert: props.onPrepareFileRevert,
          onCommitFileRevert: props.onCommitFileRevert,
          onStop: props.port?.stopToolCall && item.tool.id ? () => void props.port!.stopToolCall!(item.tool.id!).catch(() => {}) : undefined,
        });
        const chart = inActivity ? null : chartOfTool(item.tool);
        return chart
          ? h(Boundary, { retryKey: item.tool.args, fallback: toolCard }, () => h(ChartCard, { spec: chart, callId: item.tool.id }))
          : toolCard;
      }
      case "reads":
        return h(ReadsCard, { tools: item.tools });
      case "guardian":
        return h(GuardianCard, { g: item.g });
      case "approval":
        return h(ApprovalCard, { item, onApprove: props.onApprove, onFullAccess: props.onFullAccess, onPlan: props.onPlan });
      case "ask":
        return item.ask.origin
          ? h(ElicitCard, { item, onAnswer: props.onAnswer })
          : h(AskCard, { item, onAnswer: props.onAnswer, onEngage: props.port?.engageQuestion?.bind(props.port), refine: props.port?.refinePrompt?.bind(props.port) });
      case "compaction":
        return h(CompactionCard, { c: item.c, done: item.done });
      case "remember":
        return h(RememberCard, { m: item.m, forgotten: item.forgotten, onForget: (name: string) => props.onForget(item.id, name) });
      case "worked":
        return h(WorkedLine, { ms: item.ms, startedAt: item.startedAt });
      case "extension":
        return h(ExtensionCard, { ext: item.ext, onInvoke: props.onExtInvoke, onSubmit: props.onExtSubmit });
      case "notice":
        return h(NoticeCard, { item, onSettings: props.onSettings });
      default:
        return null;
    }
  };

  const entry = (item: Item): VNodeChild => h("div", {
    class: "chunk",
    key: item.id,
  }, [
    h("div", {
      class: "enterbox",
      "data-item": item.id,
      "data-enter": owed.value.has(item.id) ? "" : undefined,
      "data-call": item.t === "tool" && item.tool.id ? item.tool.id : undefined,
    }, [card(item)]),
  ]);

  return () => {
    const paging = props.historyPaging;
    const empty = props.items.length === 0 && !morePages.value && !pageBusy.value && !pageFailed.value;
    return h("div", {
      class: "scroll",
      "data-pane": "flow",
      ref: props.scroll,
      hidden: props.hidden,
    }, [
      h("div", { class: "flow-edge", "aria-hidden": "true" }),
      h("div", {
        class: "flow",
        ref: flow,
        "data-empty": empty ? "" : undefined,
      }, [
        morePages.value ? h("button", {
          type: "button",
          class: "history-more",
          "data-action": "history.load-earlier",
          disabled: pageBusy.value,
          onClick: () => void paging?.more().catch(() => {}),
        }, pageBusy.value ? t("common.loading") : t("chat.load_earlier_messages")) : null,
        fold.value.cut > 0 ? h("button", {
          type: "button",
          class: "fold-earlier",
          "data-action": "transcript.unfold-compacted",
          "aria-expanded": unfolded.value,
          onClick: () => { unfolded.value = !unfolded.value; },
        }, [
          h("i", { "aria-hidden": "true" }),
          h("span", t("chat.n_earlier_messages_folded", { n: fold.value.folded })),
          h("b", t(unfolded.value ? "chat.hide_the_folded_content" : "chat.show_the_folded_content")),
        ]) : null,
        empty ? h("div", { class: "hero empty" }, [
          h(StudioIcon, { name: "chat" }),
          h("p", { class: "t" }, t(props.needsProject ? "chat.open_a_project_first" : "chat.describe_the_task_reasonix_takes")),
          h("p", { class: "s" }, t(props.needsProject ? "chat.reading_code_running_tests_and" : "chat.it_can_read_code_research")),
          props.needsProject ? h("div", { class: "herogo" }, [
            h("button", { class: "pick", type: "button", "data-action": "workspace.add", onClick: props.onOpenProject }, t("chat.open_a_project")),
            h("button", { class: "stay", type: "button", onClick: props.onKeepHere }, t("chat.stay_where_i_am")),
          ]) : null,
        ]) : null,
        ...groups.value.map((group) => group.kind === "agents"
          ? h("div", { class: "chunk agents-chunk", key: group.key }, [h(SubagentGroup, { tasks: group.tasks, port: props.port })])
          : group.kind === "activity"
            ? h("div", { class: "chunk activity-chunk", key: group.key }, [activity(group)])
            : entry(group.item)),
        props.brought && props.brought.ids.length > 0 ? h(BroughtResults, {
          items: props.items,
          ids: props.brought.ids,
          onDismiss: props.brought.dismiss,
          takeovers: props.takeovers ?? {},
          onExtInvoke: props.onExtInvoke,
          onPrepareFileRevert: props.onPrepareFileRevert,
          onCommitFileRevert: props.onCommitFileRevert,
          port: props.port as Pick<AgentPort, "stopToolCall"> | undefined,
        }) : null,
        props.waiting.ttftSince ? h(Await, { since: props.waiting.ttftSince, retry: props.waiting.retry }) : null,
        h("div", { class: "flow-end", "aria-hidden": "true" }),
      ]),
    ]);
  };
}, {
  name: "Transcript",
  props: [
    "historyPaging", "items", "port", "revision", "waiting", "scroll", "hidden", "focus", "find", "query",
    "onApprove", "onFullAccess", "onPlan", "onAnswer", "onForget", "onSettings", "onExtInvoke", "takeovers",
    "reply", "onResend", "onWithdraw", "onExtSubmit", "needsProject", "onOpenProject", "onKeepHere",
    "onSuggest", "foldWork", "turnRunning", "scope", "checkpoints", "onPrepareRewind", "onPrepareFileRevert",
    "onCommitFileRevert", "onCommitRewind", "entering", "onEntered", "brought",
  ],
});
</script>
