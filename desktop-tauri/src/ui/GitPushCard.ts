import { computed, h, onUnmounted, ref, toValue, type ComputedRef, type MaybeRefOrGetter, type VNode } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort, PushPlan, PushResult } from "../port/port";

type Phase =
  | { phase: "idle" }
  | { phase: "reading" }
  | { phase: "ready"; plan: PushPlan; pushing: boolean; error?: string }
  | { phase: "done"; result: PushResult }
  | { phase: "failed"; error: string };

// The one git action that leaves the machine, so it is always two steps: what
// would go and where is read first, with no network, and only the confirm button
// sends exactly that plan. It is never forced and never automatic.
export function usePushCard(port: AgentPort, repo: MaybeRefOrGetter<string | undefined>, onPushed: () => void): { button: ComputedRef<VNode>; card: ComputedRef<VNode | null> } {
  const state = ref<Phase>({ phase: "idle" });
  let live = true;
  onUnmounted(() => { live = false; });

  const preview = () => {
    state.value = { phase: "reading" };
    port.pushPreview(toValue(repo)).then(
      (plan) => { if (live) state.value = { phase: "ready", plan, pushing: false }; },
      (e) => { if (live) state.value = { phase: "failed", error: reason(e) }; },
    );
  };
  const close = () => { state.value = { phase: "idle" }; };

  const button = computed(() => h("button", {
    type: "button",
    class: "btn sm push-open",
    "data-action": "push.preview",
    disabled: state.value.phase !== "idle",
    onClick: preview,
  }, t("workbench.push_2")));

  const card = computed<VNode | null>(() => {
    const s = state.value;
    if (s.phase === "idle") return null;

    if (s.phase === "reading") {
      return h("section", { class: "push-card", "data-phase": "reading", "aria-label": t("workbench.push") }, [
        h("p", { class: "refine-wait", role: "status" }, t("workbench.reading_what_would_be_pushed")),
        h("footer", null, [h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "push.close", onClick: close }, t("common.cancel"))]),
      ]);
    }

    if (s.phase === "failed") {
      return h("section", { class: "push-card", "data-phase": "failed", "aria-label": t("workbench.push") }, [
        h("p", { class: "refine-error", role: "alert" }, s.error),
        h("footer", null, [
          h("button", { type: "button", class: "btn sm", "data-action": "push.preview", onClick: preview }, t("common.read_again")),
          h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "push.close", onClick: close }, t("common.close")),
        ]),
      ]);
    }

    if (s.phase === "done") {
      const { plan } = s.result;
      return h("section", { class: "push-card", "data-phase": "done", "aria-label": t("workbench.push") }, [
        h("p", { class: "commit-done", role: "status" },
          t(s.result.new ? "workbench.pushed_branch_to_host_the" : "workbench.pushed_n_commits_to_host", { branch: plan.upstreamBranch, host: plan.host, n: String(plan.ahead) })),
        h("footer", null, [h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "push.close", onClick: close }, t("common.collapse"))]),
      ]);
    }

    const { plan, pushing, error } = s;
    const confirm = () => {
      state.value = { ...s, pushing: true, error: undefined };
      const r = toValue(repo);
      port.push(r ? { repo: r, plan } : { plan }).then(
        (result) => {
          if (!live) return;
          state.value = { phase: "done", result };
          onPushed();
        },
        (e) => { if (live) state.value = { phase: "ready", plan, pushing: false, error: reason(e) }; },
      );
    };
    return h("section", { class: "push-card", "data-phase": "ready", "aria-label": t("workbench.push") }, [
      h("header", null, [
        h("b", null, t("workbench.push_to_host", { host: plan.host })),
        h("small", { "data-scheme": plan.scheme }, plan.scheme),
      ]),
      h("dl", null, [
        h("dt", null, t("common.branch")),
        h("dd", null, `${plan.branch} → ${plan.remote}/${plan.upstreamBranch}`),
        h("dt", null, t("workbench.remote")),
        h("dd", { class: "push-url", title: plan.url }, plan.url),
        h("dt", null, t("workbench.will_send")),
        h("dd", null, `${t("workbench.n_commits", { n: String(plan.ahead) })}${plan.head ? ` · ${plan.head.slice(0, 7)}` : ""}`),
      ]),
      plan.behind > 0 ? h("p", { class: "push-behind", role: "status" }, t("workbench.the_remote_has_n_commits", { n: String(plan.behind) })) : null,
      plan.ahead === 0 ? h("p", { class: "push-behind", role: "status" }, t("workbench.the_remote_already_has_every")) : null,
      error ? h("p", { class: "refine-error", role: "alert" }, error) : null,
      h("small", null, t("workbench.pushes_this_one_branch_and")),
      h("footer", null, [
        h("button", { type: "button", class: "btn sm", "data-primary": "true", "data-action": "push.confirm", disabled: pushing || plan.ahead === 0, onClick: confirm },
          pushing ? t("workbench.pushing") : t("workbench.push_to_host", { host: plan.host })),
        h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "push.close", disabled: pushing, onClick: close }, t("common.cancel")),
      ]),
    ]);
  });

  return { button, card };
}
