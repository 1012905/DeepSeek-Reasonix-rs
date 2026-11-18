<script lang="ts">
import { defineComponent, h, type PropType } from "vue";
import { t } from "../i18n";
import { currentStep, stepDone, stepLabel, type PlanStep } from "../state/session";
import RMark from "./RMark.vue";
import StudioIcon from "./StudioIcon.vue";

// Keyed by text, not index: todo_write rewrites the whole list every call, and
// under index keys row one morphs into a different sentence instead of the old
// row leaving and a new one arriving. Duplicates get a suffix so each row keeps
// one key.
export function stepKeys(steps: PlanStep[]): string[] {
  const seen = new Map<string, number>();
  return steps.map((s) => {
    const n = (seen.get(s.text) ?? 0) + 1;
    seen.set(s.text, n);
    return n === 1 ? s.text : `${s.text}#${n}`;
  });
}

export const stepIndent = (s: PlanStep) => (s.level ? Math.min(s.level, 4) * 12 : undefined);

type Mark = "done" | "now" | "todo";

/** How the list is moving: a proposal has not started, so every row is numbered
 *  whatever status the seed gives it; a step in hand turns while the run moves,
 *  is ringed while it waits on a person, and is only emphasised otherwise. */
export type StepsState = "proposed" | "moving" | "waiting" | "still";

export const PlanSteps = defineComponent({
  name: "PlanSteps",
  props: {
    steps: { type: Array as PropType<PlanStep[]>, required: true },
    state: { type: String as PropType<StepsState>, required: true },
  },
  setup(props) {
    return () => {
      if (props.steps.length === 0) return null;
      const proposed = props.state === "proposed";
      const now = proposed ? -1 : currentStep(props.steps);
      const keys = stepKeys(props.steps);
      return h("ol", { class: "plan-steps", "data-state": props.state }, props.steps.map((st, i) => {
        const mark: Mark = proposed ? "todo" : stepDone(st) ? "done" : i === now ? "now" : "todo";
        const indent = stepIndent(st);
        return h("li", {
          key: keys[i],
          class: "plan-step",
          "data-state": mark,
          style: indent === undefined ? undefined : { marginInlineStart: `${indent}px` },
        }, [
          h("span", {
            class: "plan-step-mark",
            "aria-label": mark === "done" ? t("plan.step.done") : mark === "now" ? t("plan.step.now") : t("plan.step.todo"),
          }, [
            mark === "done" ? h(StudioIcon, { name: "check" })
              : mark === "now" && props.state === "moving" ? h(RMark, { class: "plan-spin" })
                : mark === "now" && props.state === "waiting" ? null : i + 1,
          ]),
          h("span", { class: "plan-step-text" }, proposed ? st.text : stepLabel(st)),
        ]);
      }));
    };
  },
});

/** The ordinal a person reads for where the plan stands: the step in hand, or
 *  the next one to start. */
export function stepOrdinal(steps: PlanStep[]): number {
  const now = currentStep(steps);
  if (now >= 0) return now + 1;
  return Math.min(steps.filter(stepDone).length + 1, steps.length);
}

export default PlanSteps;
</script>
