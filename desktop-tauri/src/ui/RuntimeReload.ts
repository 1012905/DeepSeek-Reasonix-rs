import { computed, h, ref, toValue, watch, type MaybeRefOrGetter, type VNode } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";

// Reloading the runtime is the one action on this page with a duration: sidecars
// stop and start, and the skill/command/hook directories are walked again. The
// states below are what the control has to say for itself — offering, working,
// landed, refused — because without them the only signal a reload happened at
// all is that the app did not visibly change.
type State = "" | "run" | "ok" | "bad";

// Long enough to read, short enough that the row goes back to offering the
// action rather than sitting on a stale success.
const SETTLED_MS = 4000;

// A hook rather than a component: the button belongs in the group's header slot
// and the note in its body, and one component cannot fill two slots.
export function useRuntimeReload(port: MaybeRefOrGetter<AgentPort>, onDone: () => void) {
  const feedback = ref<{ state: State; note: string }>({ state: "", note: "" });
  let attempts = 0;
  let connection = toValue(port);

  watch(() => toValue(port), (p) => {
    if (p !== connection) {
      connection = p;
      feedback.value = { state: "", note: "" };
    }
  });

  const go = async () => {
    const attempt = ++attempts;
    const started = connection;
    feedback.value = { state: "run", note: t("app.restarting_resident_processes_and_rescanning") };
    try {
      await toValue(port).reloadExtensions();
      if (connection !== started || attempts !== attempt) return;
      feedback.value = { state: "ok", note: t("app.live_the_next_turn_runs") };
      onDone();
    } catch (e) {
      if (connection !== started || attempts !== attempt) return;
      // A refusal is the kernel's, and it knows why: a turn in flight, a
      // background job, a session that moved. Say its reason, not a generic one.
      feedback.value = { state: "bad", note: reason(e) };
    }
  };

  watch(feedback, (f) => {
    if (f.state !== "ok") return;
    const id = setTimeout(() => {
      feedback.value = { state: "", note: "" };
    }, SETTLED_MS);
    return () => clearTimeout(id);
  });

  const state = computed(() => feedback.value.state);
  const action = computed<VNode>(() => h(
    "button",
    {
      class: "act reload",
      "data-action": "extensions.reload",
      "data-s": state.value || undefined,
      disabled: state.value === "run",
      onClick: go,
    },
    [
      h("i", { class: "rdot" }),
      state.value === "run" ? t("app.reloading") : state.value === "ok" ? t("app.live") : t("app.reload_runtime"),
    ],
  ));
  const note = computed<VNode | null>(() => (feedback.value.note
    ? h("div", { class: "rnote", "data-s": state.value, role: "status" }, feedback.value.note)
    : null));

  return {
    state,
    applied: () => {
      attempts++;
      feedback.value = { state: "ok", note: t("app.live_the_next_turn_runs") };
    },
    report: (message: string) => {
      attempts++;
      feedback.value = { state: "bad", note: t("app.changes_saved_but_the_runtime", { reason: message }) };
    },
    action,
    note,
  };
}
