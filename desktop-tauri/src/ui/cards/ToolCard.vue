<script lang="ts">
import { computed, defineComponent, h, onScopeDispose, ref, watch, type PropType, type VNodeChild } from "vue";
import { useStartsOpen } from "../../state/foldpref";
import { t } from "../../i18n";
import { nestLabel, ranLabel } from "../delegation";
import type { ExtensionSurface, Tool } from "../../port/wire";
import type { RewindPlan, RewindResult } from "../../port/port";
import { categoryOf, labelFor, mcpOrigin, runLabelFor } from "../icons";
import { Sym, iconFor } from "../Sym.vue";
import { activityVerb } from "../activityEarlier";
import { GOAL_STATUS, argOf, goalUpdate, shortArgs } from "../args";
import Cost from "../Cost.vue";
import { count, seconds, tokens, bytes } from "../../i18n/format";
import { currentStep, parsePlan, stepDone } from "../../state/session";
import DiffView from "./DiffView.vue";
import BringCall from "./BringCall.vue";
import { StepWait, StopCall } from "./StopCall.vue";
import { stepSummary } from "./stepsum";
import { BackgroundNote, BackgroundReading } from "./BackgroundJob.vue";
import WaitNote from "./WaitNote.vue";
import { BoundNote, Term, ToolOutput } from "./ToolOutput.vue";
import JsonText from "./JsonText.vue";
import ExtensionView from "./ExtensionView.vue";
import { toolChangedFile, toolErrorDetail, toolFailed, toolFailureLabel, toolOutputEchoed, toolRefusalReason } from "./outcome";
import StudioIcon from "../StudioIcon.vue";
import { registerNest } from "./nestreveal";
import { useFullOutput, type ReadOutput } from "./useFullOutput";
import type { StateTool } from "../../state/outputcap";
import ImagePreview from "../ImagePreview.vue";

// The head crossfades before it swaps: colour arriving is the finish signal,
// and the transition is the only thing that may fire it.
const SWAP = 90;

function prettyArgs(args?: string): string {
  if (!args) return "";
  try {
    return JSON.stringify(JSON.parse(args), null, 2);
  } catch {
    return args;
  }
}

const shortCallId = (id?: string) => id ? id.replace(/^call_/, "").slice(-8) : "—";

const lastLine = (text?: string) => text?.split("\n").map((l) => l.trim()).filter(Boolean).at(-1) ?? "";

// How long an activity step has run or took, in whole seconds; nothing under
// one. A running step counts from when the kernel says it started.
const StepTime = defineComponent({
  name: "StepTime",
  props: { tool: { type: Object as PropType<StateTool>, required: true }, running: { type: Boolean, required: true } },
  setup(props) {
    const drawnAt = Date.now();
    const now = ref(Date.now());
    let timer: ReturnType<typeof setInterval> | undefined;
    const stop = () => { if (timer !== undefined) { clearInterval(timer); timer = undefined; } };
    watch(() => props.running, (running) => {
      stop();
      if (!running) return;
      now.value = Date.now();
      timer = setInterval(() => (now.value = Date.now()), 1000);
    }, { immediate: true });
    onScopeDispose(stop);
    return () => {
      const ms = props.running ? Math.max(0, now.value - (props.tool.startedAt || drawnAt)) : props.tool.durationMs ?? 0;
      return ms < 1000 ? null : h("span", { class: "step-sum" }, t("common.n_s", { n: Math.round(ms / 1000) }));
    };
  },
});

// A step that turns failed under the reader shakes once; one drawn failed does not.
function useFellInto(bad: () => boolean) {
  const on = ref(false);
  let was = bad();
  let timer: number | undefined;
  watch(bad, (next) => {
    const fell = !was && next;
    was = next;
    if (!fell) return;
    on.value = true;
    timer = window.setTimeout(() => (on.value = false), 240);
  });
  onScopeDispose(() => { if (timer !== undefined) window.clearTimeout(timer); });
  return on;
}

function useSettling(running: () => boolean) {
  const pop = ref(false);
  const swap = ref(false);
  let was = running();
  let a: number | undefined;
  let b: number | undefined;
  watch(running, (next) => {
    const fell = was && !next;
    was = next;
    if (!fell) return;
    pop.value = true;
    swap.value = true;
    if (a !== undefined) window.clearTimeout(a);
    if (b !== undefined) window.clearTimeout(b);
    a = window.setTimeout(() => (swap.value = false), SWAP);
    b = window.setTimeout(() => (pop.value = false), 340);
  });
  onScopeDispose(() => {
    if (a !== undefined) window.clearTimeout(a);
    if (b !== undefined) window.clearTimeout(b);
  });
  return { pop, swap };
}

const NESTED_PREVIEW = 400;
const NESTED_WHOLE_CAP = 200_000;

// An identifier reads correctly in the mono tag and nowhere else. A shell call
// spends it on the interpreter; otherwise it names the MCP tool that answered.
const tagFor = (tool: Tool): string | null => {
  const ex = tool.execution;
  if (ex?.kind === "shell" && ex.shell) return ex.shell;
  return mcpOrigin(tool.resolvedName || tool.name)?.tool ?? null;
};

const SHELL_HINT: Record<string, string> = {
  bash: "命令交给 bash 执行",
  "git-bash": "命令交给 Git Bash 执行",
  pwsh: "命令交给 PowerShell 7 执行 —— 语法是 PowerShell，不是 bash",
  powershell: "本机没有 bash，命令交由 Windows PowerShell 执行 —— 它不支持 && 和 ||",
};

const tagHint = (tool: Tool) => {
  const ex = tool.execution;
  return ex?.kind === "shell" && ex.shell ? SHELL_HINT[ex.shell] : undefined;
};

// Only the categories the spec gives a colour to; the rest stay neutral.
const KINDED = new Set(["net", "deleg", "write", "mcp", "mem"]);

const childTokens = (kids: Tool[]) => kids.reduce((n, k) => n + (k.contextTokens ?? 0), 0);

export const NestedCall = defineComponent({
  name: "NestedCall",
  props: {
    tool: { type: Object as PropType<StateTool>, required: true },
    whole: { type: Boolean, default: false },
    read: { type: Function as PropType<ReadOutput | undefined>, default: undefined },
  },
  setup(props) {
    const all = ref(false);
    const live = new Proxy(props.tool, {
      get: (_target, key) => (props.tool as unknown as Record<PropertyKey, unknown>)[key as string],
    }) as StateTool;
    const full = useFullOutput(live, props.read);
    return () => {
      const tool = props.tool;
      const body = full.text ?? "";
      const shown = tool.resolvedName || tool.name;
      const tag = tagFor(tool);
      const bad = toolFailed(tool);
      const clipped = !props.whole && body.length > NESTED_PREVIEW;
      const echoed = toolOutputEchoed(tool);
      const refusal = toolRefusalReason(tool);
      const errorDetail = toolErrorDetail(tool);
      return h("div", { class: "call", "data-call": tool.id || undefined, "data-k": KINDED.has(categoryOf(shown)) ? categoryOf(shown) : undefined }, [
        h("div", { class: "g" }, h(Sym, { icon: iconFor(shown) })),
        h("div", { class: "c" }, [
          h("div", { class: "hl" }, [
            h("span", { class: "nm", title: shown }, labelFor(shown)),
            tag ? h("span", { class: "tag", title: tagHint(tool) }, tag) : null,
            tool.args ? h("span", { class: "arg", title: prettyArgs(tool.args) }, shortArgs(tool.args)) : null,
            bad ? h("span", { class: "fail" }, toolFailureLabel(tool)) : null,
            h(Cost, { tools: [tool] }),
          ]),
          tool.output && !echoed
            ? h("div", { class: "out" }, [
              full.note,
              h(Term, { text: clipped && !all.value ? body.slice(0, NESTED_PREVIEW) : body.slice(0, NESTED_WHOLE_CAP) }),
              (props.whole || all.value) && body.length > NESTED_WHOLE_CAP ? h("div", { class: "bound bad" }, t("chat.output_too_long_showing_the")) : null,
              clipped
                ? h("div", { class: "bound" }, [
                  all.value ? null : t("chat.showing_only_the_first_400"),
                  h("button", {
                    type: "button", class: "out-more", "data-action": "tool.show-all", "aria-expanded": all.value,
                    onClick: () => (all.value = !all.value),
                  }, all.value ? t("common.collapse") : t("chat.show_all_2")),
                ])
                : null,
            ])
            : null,
          tool.err
            ? h("div", { class: "out" }, [
              refusal ? h("div", { class: "txt", "data-refusal": tool.refusalCode }, refusal) : null,
              errorDetail ? h("div", { class: "txt bad" }, errorDetail) : null,
            ])
            : null,
        ]),
      ]);
    };
  },
});

// The plan is the one payload the flow shows twice on purpose: the rail tracks
// it for the rest of the turn, the card records what it was when written.
const Steps = defineComponent({
  name: "ToolSteps",
  props: { tool: { type: Object as PropType<Tool>, required: true } },
  setup(props) {
    return () => {
      const steps = parsePlan(props.tool);
      if (!steps?.length) return h("span", { class: "fold" }, t("chat.the_plan_is_in_the"));
      const now = currentStep(steps);
      return h("div", { class: "steps" }, steps.map((st, i) => h("div", {
        class: "s", key: i,
        "data-done": stepDone(st) ? "" : undefined,
        "data-now": i === now ? "" : undefined,
      }, [
        h("span", { class: "b" }, stepDone(st) ? h(StudioIcon, { name: "check" }) : i + 1),
        h("span", { class: "t" }, h("span", { class: "ln" }, st.text)),
      ])));
    };
  },
});

const ToolVideos = defineComponent({
  name: "ToolVideos",
  props: { videos: { type: Array as PropType<NonNullable<Tool["videos"]>>, default: undefined } },
  setup(props) {
    return () => {
      if (!props.videos?.length) return null;
      return h("div", { class: "tvideos" }, props.videos.map((v, i) => h("div", { key: v.name + i, class: "tvideo" }, t("chat.video_name_detail_sent_to", {
        name: v.name,
        detail: [bytes(v.size), v.durationMs ? seconds(v.durationMs) : ""].filter(Boolean).join(" · "),
        model: v.model,
      }))));
    };
  },
});

const ToolShots = defineComponent({
  name: "ToolShots",
  props: { images: { type: Array as PropType<string[]>, default: undefined } },
  setup(props) {
    return () => {
      if (!props.images?.length) return null;
      return h("div", { class: "tshots" }, props.images.map((src, i) => h(ImagePreview, {
        key: src + i, src, name: t("chat.tool_screenshot_n", { n: i + 1 }), className: "tshot",
      }, () => h("img", { src, alt: t("chat.tool_screenshot_n", { n: i + 1 }), loading: "lazy" }))));
    };
  },
});

function changeCounts(tool: Tool): { added: number; removed: number } | null {
  if (!toolChangedFile(tool)) return null;
  const lines = tool.diff?.split("\n") ?? [];
  const added = tool.added ?? lines.filter((line) => line.startsWith("+") && !line.startsWith("+++")).length;
  const removed = tool.removed ?? lines.filter((line) => line.startsWith("-") && !line.startsWith("---")).length;
  return { added, removed };
}

// useFullOutput settles `hidden` when the region mounts; keying by it remounts
// the region when the output cap lands, while the proxy keeps a live call's
// streamed text flowing without a remount.
const Filled = defineComponent({
  name: "ToolCardOutput",
  props: {
    tool: { type: Object as PropType<StateTool>, required: true },
    read: { type: Function as PropType<ReadOutput | undefined>, default: undefined },
    shown: { type: String, required: true },
    omitted: { type: Number, default: 0 },
    id: { type: String as PropType<string | undefined>, default: undefined },
    bound: { type: Object as PropType<Tool["bound"]>, default: undefined },
  },
  setup(p) {
    const live = new Proxy(p.tool, {
      get: (_target, key) => (p.tool as unknown as Record<PropertyKey, unknown>)[key as string],
    }) as StateTool;
    const full = useFullOutput(live, p.read);
    return (): VNodeChild[] => [
      p.omitted > 0 ? h("div", { class: "bound" }, t("chat.n_characters_before_this_are", { n: count(p.omitted) })) : null,
      full.note,
      h(ToolOutput, { name: p.shown, text: full.text ?? "", bound: p.bound, id: p.id }),
    ];
  },
});

export const ToolCard = defineComponent({
  name: "ToolCard",
  props: {
    tool: { type: Object as PropType<StateTool>, required: true },
    running: { type: Boolean, required: true },
    activity: { type: Boolean, default: false },
    children: { type: Array as PropType<StateTool[]>, default: () => [] },
    readOutput: { type: Function as PropType<ReadOutput | undefined>, default: undefined },
    omitted: { type: Number, default: 0 },
    onPrepareFileRevert: { type: Function as PropType<((path: string) => Promise<RewindPlan>) | undefined>, default: undefined },
    onCommitFileRevert: { type: Function as PropType<((planId: string, resolution?: string) => Promise<RewindResult>) | undefined>, default: undefined },
    onStop: { type: Function as PropType<(() => void) | undefined>, default: undefined },
    onBring: { type: Function as PropType<(() => void) | undefined>, default: undefined },
    pinned: { type: Boolean, default: false },
    takeover: { type: Object as PropType<ExtensionSurface | undefined>, default: undefined },
    onExtInvoke: { type: Function as PropType<((actionId: string) => void) | undefined>, default: undefined },
  },
  setup(props) {
    const shown = computed(() => props.tool.resolvedName || props.tool.name);
    const from = computed(() => mcpOrigin(shown.value));
    const verb = computed(() => activityVerb(shown.value));
    const bgJob = computed(() => props.tool.execution?.state === "background_started" ? props.tool.execution.jobId : undefined);
    const head = computed(() => (props.activity && bgJob.value
      ? t("chat.sent_to_background")
      : props.activity
        ? (from.value?.tool || (verb.value ? t(verb.value) : labelFor(shown.value)))
        : props.running ? runLabelFor(shown.value) : labelFor(shown.value)));
    // A write's payload streams as arguments, and the provider only reports how
    // many characters have landed; the JSON is unparseable until the last one.
    const streaming = computed(() => props.running && !props.tool.args && (props.tool.argChars ?? 0) > 0);
    const arg = computed(() => (props.tool.name === "todo_write" ? "" : streaming.value ? `${tokens(props.tool.argChars!)} 字符` : shortArgs(props.tool.args ?? "")));
    const bad = computed(() => toolFailed(props.tool));
    const badLabel = computed(() => toolFailureLabel(props.tool));
    const refusal = computed(() => toolRefusalReason(props.tool));
    const errorDetail = computed(() => toolErrorDetail(props.tool));
    const tag = computed(() => tagFor(props.tool));
    const who = computed(() => props.tool.profile?.name?.trim());
    const settling = useSettling(() => props.running);
    const shaking = useFellInto(() => bad.value);
    // A refused call carries the same sentence twice: the kernel writes it to
    // the model as output and to the reader as an error.
    const echoed = computed(() => toolOutputEchoed(props.tool));
    const goal = computed(() => (props.tool.name === "update_goal" ? goalUpdate(props.tool.args) : null));
    const start = useStartsOpen("steps", false, () => bad.value && !props.activity, () => toolChangedFile(props.tool));
    const touched = ref<boolean | null>(props.pinned ? true : null);
    watch(bad, (next) => { if (next) touched.value = null; }, { immediate: true });
    const open = computed(() => touched.value ?? start.value);
    // A finished, closed card does not mount its body; once opened it stays, so
    // what the reader did inside survives closing it again.
    const opened = ref(false);
    watch(open, (now) => { if (now) opened.value = true; }, { immediate: true });
    const nestMounted = computed(() => open.value || props.running || opened.value);
    watch([nestMounted, () => props.children], ([mounted, children], _old, onCleanup) => {
      if (mounted || children.length === 0) return;
      onCleanup(registerNest(children.map((c) => c.id).filter((id): id is string => !!id), () => (touched.value = true)));
    }, { immediate: true });
    const changes = computed(() => changeCounts(props.tool));
    const sum = computed(() => (props.activity && !props.running ? stepSummary(props.tool, verb.value, bad.value) : ""));
    // A host refusal says why in its own words; a command's last output line
    // says more than the generic error that it exited.
    const failLine = computed(() => (props.activity && bad.value
      ? (refusal.value || (props.tool.refusalCode && errorDetail.value) || (props.tool.execution ? lastLine(props.tool.output) : "") || errorDetail.value || props.tool.err || lastLine(props.tool.output) || "")
      : ""));
    const activityArgs = computed(() => (props.activity ? prettyArgs(props.tool.args) : ""));
    const hasBody = computed(() => Boolean(
      props.takeover || props.tool.diff || goal.value || props.tool.argumentNormalization || props.tool.name === "todo_write" ||
      (props.tool.output && !echoed.value && props.children.length === 0) || props.tool.err || props.children.length || (props.activity && props.running && activityArgs.value),
    ));
    const activityResult = computed(() => Boolean(
      props.takeover || props.tool.diff || goal.value || props.tool.name === "todo_write" || props.tool.output || props.tool.err || props.children.length,
    ));

    const onToggle = (event: Event) => {
      const el = event.currentTarget as HTMLDetailsElement;
      if (el.open !== open.value) touched.value = el.open;
    };

    const heading = () => {
      const tool = props.tool;
      const changesNow = changes.value;
      const stateKey = props.running ? "common.running" : bad.value ? "common.failed" : "common.done_2";
      return [
        props.activity ? h("span", { class: "kind", "aria-hidden": "true" }, h(Sym, { icon: iconFor(shown.value) })) : null,
        h("span", { class: props.running ? "nm shim" : "nm", title: shown.value }, head.value),
        who.value ? h("span", { class: "who", title: t("chat.a_subagent_running_under_the", { name: who.value }) }, who.value) : null,
        from.value ? h("span", { class: "src", title: t("chat.a_tool_provided_by_the", { name: from.value.server }) }, from.value.server) : null,
        tag.value ? h("span", { class: "tag", title: tagHint(tool) }, tag.value) : null,
        arg.value ? h("span", { class: streaming.value ? "arg shim" : "arg", title: streaming.value ? undefined : prettyArgs(tool.args) }, arg.value) : null,
        changesNow
          ? h("span", { class: "tool-delta", "aria-label": t("chat.added_lines_added_removed_removed", changesNow) }, [
            h("b", null, `+${changesNow.added}`),
            h("i", null, `−${changesNow.removed}`),
          ])
          : null,
        bad.value && !sum.value ? h("span", { class: "fail" }, badLabel.value) : null,
        bgJob.value ? h(BackgroundReading, { id: bgJob.value }) : null,
        sum.value
          ? h("span", { class: "step-sum" }, sum.value)
          : props.activity
            ? (!changesNow ? h(StepTime, { tool, running: props.running }) : null)
            : h(Cost, { tools: [tool], running: props.running }),
        h("span", {
          class: "tool-state",
          "data-state": props.running ? "running" : bad.value ? "failed" : "done",
          "data-pop": settling.pop.value && !bad.value ? "" : undefined,
          "aria-label": t(stateKey),
          title: t(stateKey),
        }, props.running && props.activity
          ? h("i", { class: "step-spin" })
          : h(StudioIcon, { name: props.running ? "clock" : bad.value ? (props.activity ? "fail" : "warning") : "check" })),
        props.onBring && tool.id && !props.running && hasBody.value ? h(BringCall, { onBring: props.onBring }) : null,
        hasBody.value ? h("span", { class: "tool-fold", "aria-hidden": "true" }) : null,
      ];
    };

    const body = () => {
      const tool = props.tool;
      const ran = ranLabel(tool.profile);
      return h("div", { class: "out" }, [
        tool.argumentNormalization
          ? h("section", { class: "activity-call-input" }, [
            h("p", null, t("chat.argument_format_automatically_normalized")),
            h("details", null, [
              h("summary", null, t("chat.original_input_before_normalization")),
              h("pre", null, prettyArgs(tool.argumentNormalization.original)),
            ]),
          ])
          : null,
        props.activity
          ? [
            h("div", { class: "activity-call-meta" }, [
              h("span", null, t("chat.call_id", { id: shortCallId(tool.id) })),
              h("span", null, t(props.running ? "common.running" : bad.value ? "common.failed" : "common.done_2")),
            ]),
            activityArgs.value
              ? h("section", { class: "activity-call-input" }, [
                h("header", null, [h("b", null, t("common.input")), h("span", null, "JSON")]),
                h("pre", null, h(JsonText, { text: activityArgs.value })),
              ])
              : null,
            props.running && !tool.output && !tool.err && props.children.length === 0 ? h(WaitNote, { args: tool.args }) : null,
            activityResult.value
              ? h("div", { class: "activity-call-result" }, [
                h("b", null, t("chat.result")),
                tool.output
                  ? h("button", {
                    type: "button", "data-action": "tool.copy-output",
                    onClick: () => void navigator.clipboard.writeText(tool.output ?? "").catch(() => {}),
                  }, t("common.copy"))
                  : null,
              ])
              : null,
          ]
          : null,
        // A takeover replaces what the body shows, never the frame around it.
        props.takeover
          ? [
            h(ExtensionView, { body: props.takeover.view?.body ?? [], onAction: (id: string) => props.onExtInvoke?.(id) }),
            h("div", { class: "drawnby" }, t("chat.drawn_by_name", { name: props.takeover.pluginId })),
          ]
          : [
            tool.diff
              ? h(DiffView, { diff: tool.diff, path: argOf(tool.args, "path", "file_path"), onPrepare: props.onPrepareFileRevert, onCommit: props.onCommitFileRevert })
              : null,
            !tool.diff && tool.outputDiff && tool.output
              ? [h(DiffView, { diff: tool.output }), h(BoundNote, { bound: tool.bound })]
              : null,
            !tool.diff && !tool.outputDiff && tool.name === "todo_write" ? h(Steps, { tool }) : null,
            goal.value
              ? h("div", { class: "goalup", "data-s": GOAL_STATUS[goal.value.status]?.[1] ?? "run" }, [
                h("span", { class: "st" }, t(GOAL_STATUS[goal.value.status]?.[0] ?? goal.value.status)),
                goal.value.reason ? h("span", { class: "rs" }, goal.value.reason) : null,
              ])
              : null,
            !tool.diff && !tool.outputDiff && !goal.value && tool.name !== "todo_write" && tool.output && !echoed.value && props.children.length === 0
              ? h(Filled, {
                key: `out:${tool.id ?? ""}:${tool.hidden ?? 0}`,
                tool, read: props.readOutput, shown: shown.value, omitted: props.omitted, id: tool.id, bound: tool.bound,
              })
              : null,
            h(ToolShots, { images: tool.images }),
            h(ToolVideos, { videos: tool.videos }),
          ],
        // The error stays outside the takeover: an extension may redraw what a
        // call produced, never whether it failed.
        refusal.value ? h("div", { class: "txt", "data-refusal": tool.refusalCode }, refusal.value) : null,
        errorDetail.value ? h("div", { class: "txt bad" }, errorDetail.value) : null,
        props.children.length > 0
          ? h("div", { class: "nest" }, [
            h("div", { class: "nest-hd" }, [
              h("i", { class: "pip" }),
              h("span", { class: "who" }, nestLabel(who.value, tool.profile?.count, props.children.length)),
              h("span", { class: "prof" }, t("chat.its_own_context_stays_out")),
              ran ? h("span", { class: "ran", title: ran }, ran) : null,
              h("span", { class: "rt" }, [
                tool.durationMs ? seconds(tool.durationMs) : "",
                childTokens(props.children) ? tokens(childTokens(props.children)) : "",
              ].filter(Boolean).join(" · ")),
            ]),
            nestMounted.value
              ? [
                h("div", { class: "nest-bd" }, props.children.map((c) => h(NestedCall, {
                  key: `${c.id ?? ""}:${c.hidden ?? 0}`, tool: c, read: props.readOutput,
                }))),
                tool.output ? h("div", { class: "nest-ret" }, tool.output) : null,
              ]
              : null,
          ])
          : null,
      ]);
    };

    return () => h("div", {
      class: "call",
      "data-call": props.tool.id || undefined,
      "data-k": KINDED.has(categoryOf(shown.value)) ? categoryOf(shown.value) : undefined,
      "data-running": props.running ? "" : undefined,
      "data-shake": shaking.value ? "" : undefined,
    }, [
      h("div", { class: "g" }, h(Sym, { icon: iconFor(shown.value), done: settling.pop.value && !bad.value })),
      h("div", { class: "c" }, [
        hasBody.value
          ? h("details", {
            class: "tool-disclosure",
            "data-lazy": nestMounted.value ? undefined : "",
            open: open.value,
            onToggle,
          }, [
            h("summary", { class: "hl", "data-swap": settling.swap.value ? "" : undefined }, heading()),
            nestMounted.value ? body() : null,
          ])
          : h("div", { class: "hl", "data-swap": settling.swap.value ? "" : undefined }, heading()),
        failLine.value
          ? h("div", { class: "step-err", role: "status" }, [
            h("code", null, failLine.value),
            hasBody.value && !open.value
              ? h("button", { type: "button", class: "step-stop", "data-action": "tool.full-output", onClick: () => (touched.value = true) }, t("chat.see_full_output"))
              : null,
          ])
          : null,
        props.running && props.activity
          ? h(StepWait, { onStop: props.onStop && props.tool.id ? props.onStop : undefined, quiet: !!props.tool.output || props.children.length > 0 })
          : null,
        props.running && !props.activity && props.onStop && props.tool.id ? h(StopCall, { onStop: props.onStop }) : null,
        bgJob.value ? h(BackgroundNote, { id: bgJob.value }) : null,
      ]),
    ]);
  },
});

export default ToolCard;
</script>
