import { computed, h, onScopeDispose, ref, toValue, useId, type ComputedRef, type MaybeRefOrGetter, type VNode } from "vue";
import type { AgentPort } from "../port/port";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import StudioIcon from "./StudioIcon.vue";

type Refine =
  | { phase: "idle" }
  | { phase: "working"; draft: string }
  | { phase: "ready"; draft: string; text: string }
  | { phase: "failed"; draft: string; error: string };

// A rewrite of the draft, offered rather than applied: the draft stays in the
// box until the person takes the rewrite, and leaving it costs nothing.
export function usePromptRefine(
  port: MaybeRefOrGetter<Pick<AgentPort, "refinePrompt">>,
  text: MaybeRefOrGetter<string>,
  adopt: (next: string) => void,
): { button: ComputedRef<VNode>; card: ComputedRef<VNode | null>; onKey: (event: KeyboardEvent) => boolean } {
  const state = ref<Refine>({ phase: "idle" });
  let pending: AbortController | null = null;
  const tipId = useId();
  onScopeDispose(() => pending?.abort());

  const run = (draft: string) => {
    if (!draft.trim()) return;
    pending?.abort();
    const controller = new AbortController();
    pending = controller;
    state.value = { phase: "working", draft };
    toValue(port).refinePrompt(draft, controller.signal).then(
      (out) => { if (!controller.signal.aborted) state.value = { phase: "ready", draft, text: out }; },
      (e) => { if (!controller.signal.aborted) state.value = { phase: "failed", draft, error: reason(e) || t("composer.refining_failed_try_again") }; },
    );
  };
  const dismiss = () => {
    pending?.abort();
    pending = null;
    state.value = { phase: "idle" };
  };
  const take = () => {
    if (state.value.phase !== "ready") return;
    adopt(state.value.text);
    dismiss();
  };

  // Ctrl+Shift+E asks from the keyboard; Escape puts an open card away before
  // it reaches anything else the box does with it.
  const onKey = (event: KeyboardEvent): boolean => {
    if (event.key.toLowerCase() === "e" && event.shiftKey && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      run(toValue(text));
      return true;
    }
    if (event.key === "Escape" && state.value.phase !== "idle") {
      event.preventDefault();
      dismiss();
      return true;
    }
    return false;
  };

  const working = computed(() => state.value.phase === "working");
  const button = computed(() => h("button", {
    type: "button",
    class: "mode plain studio-attach studio-refine",
    "data-action": "prompt.refine",
    "aria-label": t("composer.refine_prompt"),
    "aria-describedby": tipId,
    "aria-busy": working.value,
    "aria-disabled": !toValue(text).trim() || working.value,
    onClick: () => { if (!working.value) run(toValue(text)); },
  }, [
    h(StudioIcon, { name: "spark" }),
    h("span", { class: "studio-control-tip", id: tipId, role: "tooltip" }, [
      h("b", null, t("composer.refine_prompt")),
      h("span", null, [
        t("composer.rewrite_it_more_clearly_with") + " ",
        h("span", { style: { whiteSpace: "nowrap" } }, "· Ctrl+Shift+E"),
      ]),
    ]),
  ]));

  const card = computed<VNode | null>(() => {
    const s = state.value;
    if (s.phase === "idle") return null;
    return h("section", { class: "refine-card", "data-phase": s.phase, "aria-label": t("composer.refined_prompt") }, [
      h("header", null, [
        h(StudioIcon, { name: "spark" }),
        h("b", null, t("composer.refined_prompt")),
        h("small", null, s.phase !== "working" && s.draft !== toValue(text)
          ? t("composer.the_box_has_changed_adopting")
          : t("composer.refine_note")),
      ]),
      s.phase === "working" && h("p", { class: "refine-wait", role: "status" }, [
        h(StudioIcon, { name: "spark" }),
        t("composer.refining"),
        h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "prompt.discard", onClick: dismiss }, t("common.cancel")),
      ]),
      s.phase === "failed" && h("p", { class: "refine-error", role: "alert" }, s.error),
      s.phase === "ready" && h("div", { class: "refine-text" }, s.text),
      s.phase !== "working" && h("footer", null, [
        h("s", { class: "refine-original" }, t("composer.refine_original", { text: s.draft })),
        h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "prompt.discard", onClick: dismiss }, t("composer.discard")),
        h("button", { type: "button", class: "btn", "data-action": "prompt.refine", onClick: () => run(s.draft) },
          s.phase === "failed" ? t("common.retry") : t("composer.try_again")),
        s.phase === "ready" && h("button", { type: "button", class: "btn", "data-primary": "", "data-action": "prompt.adopt", onClick: take }, t("composer.adopt")),
      ]),
    ]);
  });

  return { button, card, onKey };
}
