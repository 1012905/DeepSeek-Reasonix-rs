<script lang="ts">
import { computed, defineComponent, h, useId, type PropType } from "vue";
import { t } from "../i18n";
import type { AgentPort, WorkspaceGit } from "../port/port";
import { Picker, type MenuItem } from "./Menu.vue";
import StudioIcon from "./StudioIcon.vue";
import { useWorkspaceBranches } from "./useWorkspaceBranches";

// Sizing belongs with the pane-aware positioning below; the stylesheet owns
// the card's visual treatment and hover/focus states.
const cardStyle = {
  width: "max-content",
  minWidth: 0,
  maxWidth: "min(260px, calc(100vw - 24px))",
  whiteSpace: "normal",
  overflowWrap: "anywhere",
};

function fitCard(anchor: HTMLElement) {
  const card = anchor.querySelector<HTMLElement>(".studio-branch-card");
  if (!card) return;
  const box = anchor.getBoundingClientRect();
  const pane = anchor.closest(".pane")?.getBoundingClientRect();
  const scale = anchor.offsetWidth ? box.width / anchor.offsetWidth : 1;
  const left = Math.max(0, pane?.left ?? 0) + 12 * scale;
  const right = Math.min(innerWidth, pane?.right ?? innerWidth) - 12 * scale;
  card.style.maxWidth = `${Math.max(0, Math.min(260, (right - left) / scale))}px`;
  const width = card.getBoundingClientRect().width;
  const offset = (Math.max(left, Math.min(box.left, right - width)) - box.left) / scale;
  card.style.left = `${offset}px`;
  card.style.setProperty("--branch-tip-anchor", `${15 - offset}px`);
}

// The composer's branch chip: the workspace's branch as git itself names it,
// and the menu that moves the session to another one. The reading lives in
// the pane's tree refresh; this component owns only the choosing.
export default defineComponent({
  name: "BranchChip",
  props: {
    port: { type: Object as PropType<AgentPort>, required: true },
    // The work tree's git answer (/workspace/git), refreshed on the pane's tree
    // path. null is "not answered yet" and renders nothing; repo:false renders
    // a chip that says so — three states that must not read as one word.
    git: { type: null as unknown as PropType<WorkspaceGit | null>, required: true },
    changeCount: { type: Number, required: true },
    // The count is drawn elsewhere (the changed-files pill), so the chip keeps
    // only the branch name.
    countElsewhere: { type: Boolean, default: false },
    // Called when a switch lands (status refresh) and when it moved the tree
    // (the change list, the branch reading and the workspace view re-read).
    onChanged: { type: Function as PropType<() => void>, required: true },
    onSwitched: { type: Function as PropType<(git: WorkspaceGit) => void> },
    onError: { type: Function as PropType<(e: unknown) => void>, required: true },
  },
  setup(props) {
    const branchTipId = useId();
    const { locals, moving, loadBranches, switchTo } = useWorkspaceBranches(
      () => props.port,
      () => props.onChanged(),
      (git) => props.onSwitched?.(git),
      (error) => props.onError(error),
    );
    const busy = computed(() => moving.value !== null);
    // The list re-reads on open: a terminal or another window can move HEAD
    // while this pane is not looking, and a stale list would offer a switch
    // the repository no longer means.
    // A branch another worktree holds is listed but closed: git would refuse
    // the switch, so the row says where the branch lives instead of failing.
    const branchItems = computed<MenuItem[]>(() => [
      { value: "__branches", label: t("composer.local_branches"), right: String(locals.value.length), header: true },
      ...locals.value.map((b) => ({
        value: b.name,
        label: b.name,
        disabled: !!b.worktree,
        desc: b.worktree ? t("composer.checked_out_in_another_worktree") : undefined,
      })),
    ]);

    const fit = (event: Event) => fitCard(event.currentTarget as HTMLElement);

    return () => {
      const git = props.git;
      if (git && !git.repo && git.absent) {
        const refused = git.absent === "refused";
        const label = refused ? t("composer.git_refused_to_open_it") : t("composer.git_did_not_respond");
        return h("div", { class: "studio-branch-pop", onMouseenter: fit, onFocusin: fit }, [
          h("div", { class: "mode plain studio-branch", "data-norepo": "", "data-absent": git.absent, role: "img", "aria-label": label, tabindex: 0, "aria-describedby": branchTipId }, [
            h("span", { class: "ic", "aria-hidden": "true" }, [h(StudioIcon, { name: "branch" })]),
            h("span", { class: "lb" }, label),
          ]),
          h("div", { class: "studio-branch-card", style: cardStyle, id: branchTipId, role: "tooltip" }, [
            h("b", {}, refused ? t("composer.this_folder_has_a_git") : t("composer.this_folder_has_a_git_2")),
            h("span", {}, refused ? (git.detail ? t("composer.git_said_detail", { detail: git.detail }) : t("composer.run_git_status_in_a")) : t("composer.branches_cannot_be_shown_for")),
          ]),
        ]);
      }
      if (git && !git.repo) {
        return h("div", { class: "studio-branch-pop", onMouseenter: fit, onFocusin: fit }, [
          h("div", { class: "mode plain studio-branch", "data-norepo": "", role: "img", "aria-label": t("composer.no_git_repository"), tabindex: 0, "aria-describedby": branchTipId }, [
            h("span", { class: "ic", "aria-hidden": "true" }, [h(StudioIcon, { name: "branch" })]),
            h("span", { class: "lb" }, t("composer.no_git_repository")),
          ]),
          h("div", { class: "studio-branch-card", style: cardStyle, id: branchTipId, role: "tooltip" }, [
            h("b", {}, t("composer.this_workspace_is_not_version")),
            h("span", {}, t("composer.there_is_no_git_repository")),
          ]),
        ]);
      }
      if (!git?.repo) return null;
      return h("div", { class: "studio-branch-pop", onMouseenter: fit, onFocusin: fit }, [
        h(Picker, {
          className: "mode plain studio-branch",
          "data-action": "git.branch",
          ariaLabel: t("composer.current_git_branch_branch", { branch: git.branch }),
          ariaDescribedBy: branchTipId,
          place: "bottom",
          current: git.detached ? "" : git.branch,
          pending: busy.value,
          items: branchItems.value,
          onOpen: loadBranches,
          onPick: switchTo,
          label: [
            h("span", { class: "ic", "aria-hidden": "true" }, [h(StudioIcon, { name: "branch" })]),
            h("span", { class: "lb" }, moving.value === null ? git.branch : t("composer.switching_to_branch", { branch: moving.value })),
            props.changeCount > 0 && !props.countElsewhere && !busy.value && h("small", {}, t("composer.n_changes", { n: props.changeCount })),
            h(StudioIcon, { name: "down" }),
          ],
        }),
        h("div", { class: "studio-branch-card", style: cardStyle, id: branchTipId, role: "tooltip" }, [
          h("b", {}, git.detached ? t("composer.detached_head_sha", { sha: git.branch }) : t("composer.current_branch_branch", { branch: git.branch })),
          h("span", {}, props.changeCount > 0 ? t("composer.current_workspace_n_local_changes", { n: props.changeCount }) : t("composer.current_workspace_future_tasks_continue")),
          h("small", {}, t("composer.refreshes_on_turns_and_writes")),
        ]),
      ]);
    };
  },
});
</script>
