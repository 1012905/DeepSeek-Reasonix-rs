<script lang="ts">
import { Fragment, defineComponent, h, inject, ref, type InjectionKey, type PropType, type VNodeChild } from "vue";
import { t } from "../../i18n";
import { elapsed } from "../../i18n/format";
import type { Item } from "../../state/session";
import type { AgentPort } from "../../port/port";
import { delegateStatus, isDelegation, type DelegateStatus } from "../delegation";
import { activityVerb } from "../activityEarlier";
import { shortArgs } from "../args";
import { useWatched } from "../Cost.vue";
import RMark from "../RMark.vue";
import StudioIcon from "../StudioIcon.vue";

type Stopper = Partial<Pick<AgentPort, "stopToolCall">>;
type Task = Extract<Item, { t: "tool" }>;

/** Opens a delegate's own run in the workbench; resolves false when it cannot. */
export const AgentRunOpener: InjectionKey<((callId: string) => Promise<boolean>) | null> = Symbol("AgentRunOpener");

const isAgent = (item: Item): item is Task => item.t === "tool" && !item.tool.parentId && isDelegation(item.tool);

/** A run of work, split where two or more delegates were sent out side by side:
 *  those read as one group of their own, everything else as the execution group. */
export function workLanes(items: Item[]): { agents: Task[] | null; items: Item[] }[] {
  const lanes: { agents: Task[] | null; items: Item[] }[] = [];
  let at = 0;
  while (at < items.length) {
    let end = at;
    while (end < items.length && isAgent(items[end])) end++;
    if (end - at >= 2) {
      lanes.push({ agents: items.slice(at, end) as Task[], items: [] });
      at = end;
      continue;
    }
    const last = lanes[lanes.length - 1];
    const take = items.slice(at, Math.max(end, at + 1));
    if (last && !last.agents) last.items.push(...take);
    else lanes.push({ agents: null, items: take });
    at += take.length;
  }
  return lanes;
}

export const WorkLanes = defineComponent({
  name: "WorkLanes",
  props: {
    items: { type: Array as PropType<Item[]>, required: true },
    port: { type: Object as PropType<Stopper | undefined>, default: undefined },
    draw: { type: Function as PropType<(items: Item[]) => VNodeChild>, required: true },
  },
  setup(props) {
    return () => workLanes(props.items).map((lane, i) => (lane.agents
      ? h(SubagentGroup, { key: lane.agents[0].id, tasks: lane.agents, port: props.port })
      : h(Fragment, { key: lane.items[0]?.id ?? i }, [props.draw(lane.items)])));
  },
});

const HUES = 3;
// The same name always gets the same colour, wherever the delegate is drawn.
export function agentHue(name: string): number {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.codePointAt(0)!) >>> 0;
  return h % HUES;
}

export const nameOf = (x: Task) => x.tool.profile?.name || "task";

export const AgentAvatar = defineComponent({
  name: "AgentAvatar",
  props: { name: { type: String, required: true } },
  setup(props) {
    return () => h("span", { class: "agent-avatar", "data-hue": agentHue(props.name), "aria-hidden": "true" }, h(StudioIcon, { name: "agent" }));
  },
});

export const SubagentGroup = defineComponent({
  name: "SubagentGroup",
  props: {
    tasks: { type: Array as PropType<Task[]>, required: true },
    port: { type: Object as PropType<Stopper | undefined>, default: undefined },
  },
  setup(props) {
    const touched = ref<boolean | null>(null);
    return () => {
      const status = props.tasks.map((x) => delegateStatus(x.running, x.tool));
      const running = status.filter((s) => s === "running").length;
      const failed = status.filter((s) => s === "failed" || s === "refused").length;
      const done = status.filter((s) => s === "done").length;
      const settled = running === 0;
      const open = touched.value ?? !(settled && failed === 0);
      const total = Math.max(0, ...props.tasks.map((x) => x.tool.durationMs ?? 0));
      const stop = props.port?.stopToolCall
        ? (x: Task) => void props.port!.stopToolCall!(x.tool.id!).catch(() => {})
        : undefined;
      const closed = settled && !open;
      return h("section", {
        class: "agent-group",
        "data-settled": settled ? "" : undefined,
        "aria-label": t("subagent.group.parallel", { n: props.tasks.length }),
      }, [
        h("header", { class: "agent-group-head" }, [
          h(StudioIcon, { name: "branch", class: "agent-group-icon" }),
          h("b", null, closed ? t("subagent.group.count", { n: props.tasks.length }) : t("subagent.group.parallel", { n: props.tasks.length })),
          closed
            ? h("span", { class: "agent-group-sum" }, [
              failed === 0 ? t("subagent.group.all_delivered", { n: done }) : t("subagent.group.delivered", { n: done }),
              total > 0 ? ` · ${elapsed(total)}` : "",
            ])
            : [
              running > 0 ? h("span", { class: "agent-pill", "data-tone": "run" }, t("subagent.group.running", { n: running })) : null,
              failed > 0 ? h("span", { class: "agent-pill", "data-tone": "err" }, t("subagent.group.failed", { n: failed })) : null,
              done > 0 ? h("span", { class: "agent-group-done" }, t("subagent.group.delivered", { n: done })) : null,
            ],
          h("span", { class: "agent-group-fill" }),
          running > 0 && stop
            ? h("button", { type: "button", class: "agent-quiet", "data-action": "agent.stop-all", onClick: () => props.tasks.filter((x) => x.running && x.tool.id).forEach(stop) }, t("common.stop_all"))
            : null,
          settled
            ? h("button", { type: "button", class: "agent-quiet", "data-action": "agent.group-fold", "aria-expanded": open, onClick: () => (touched.value = !open) }, open ? t("common.collapse") : t("common.expand"))
            : null,
        ]),
        open
          ? h("div", { class: "agent-group-body" }, props.tasks.map((x, i) => h(AgentRow, { key: x.id, task: x, status: status[i], onStop: stop })))
          : null,
      ]);
    };
  },
});

const AgentRow = defineComponent({
  name: "AgentRow",
  props: {
    task: { type: Object as PropType<Task>, required: true },
    status: { type: String as PropType<DelegateStatus>, required: true },
    onStop: { type: Function as PropType<((x: Task) => void) | undefined>, default: undefined },
  },
  setup(props) {
    const open = inject(AgentRunOpener, null);
    const watched = useWatched(() => props.status === "running");
    return () => {
      const name = nameOf(props.task);
      const steps = props.task.children.length;
      const now = props.status === "running" ? props.task.children[props.task.children.length - 1] : undefined;
      const took = props.task.tool.durationMs ? elapsed(props.task.tool.durationMs) : "";
      const reading = props.status === "running"
        ? [t("subagent.step", { n: Math.max(steps, 1) }), watched.value ? elapsed(watched.value) : ""]
        : props.status === "done"
          ? [t("subagent.state.delivered"), steps ? t("subagent.steps", { n: steps }) : "", took]
          : [props.status === "refused" ? t("subagent.state.not_run") : t("subagent.state.failed"), took];
      return h("div", { class: "agent-row", "data-status": props.status, "data-call": props.task.tool.id || undefined }, [
        h("div", { class: "agent-row-line" }, [
          h(AgentAvatar, { name }),
          h("span", { class: "agent-who" }, [
            h("b", null, name),
            props.task.tool.profile?.label ? h("span", { title: props.task.tool.profile.label }, props.task.tool.profile.label) : null,
          ]),
          h("span", { class: "agent-reading" }, [
            props.status === "running" ? h(RMark, { class: "plan-spin" }) : null,
            reading.filter(Boolean).join(" · "),
            props.status === "done" ? h(StudioIcon, { name: "check" }) : null,
            props.status === "failed" || props.status === "refused" ? h(StudioIcon, { name: "close" }) : null,
          ]),
          props.status === "running" && props.onStop && props.task.tool.id
            ? h("button", { type: "button", class: "agent-quiet", "data-action": "tool.stop", onClick: () => props.onStop?.(props.task) }, t("common.stop"))
            : null,
          open && props.task.tool.id
            ? h("button", {
              type: "button", class: "agent-open", "data-action": "agent.open", "data-target": props.task.id,
              "aria-label": t("subagent.open"), title: t("subagent.open"),
              onClick: () => void open?.(props.task.tool.id!),
            }, h(StudioIcon, { name: "panelRight" }))
            : null,
        ]),
        now
          ? h("p", { class: "agent-row-note" }, [
            t("subagent.doing", { what: t(activityVerb(now.name) ?? "subagent.run") }),
            " ",
            h("code", null, shortArgs(now.args ?? "")),
          ])
          : null,
        (props.status === "failed" || props.status === "refused") && props.task.tool.err
          ? h("p", { class: "agent-row-note", "data-tone": "err" }, props.task.tool.err)
          : null,
      ]);
    };
  },
});
export default SubagentGroup;
</script>
