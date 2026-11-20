<script lang="ts">
import { computed, defineComponent, h, ref, Teleport, useId, watch, type PropType, type VNodeChild } from "vue";
import { t } from "../i18n";
import type { WorkspaceInfo, WorktreeSelection, WorktreeSources } from "../port/workspace";
import type { AgentPort, WorkspaceGit } from "../port/port";
import { useWorkspaceBranches } from "./useWorkspaceBranches";
import StudioIcon from "./StudioIcon.vue";
import { workspaceDirectoryDescription } from "./workspaceDirectory";
import WorkspacePath, { WorkspaceBranchName, compactWorkspaceValue, workspaceName } from "./WorkspacePath.vue";
import { listenAction } from "./listen";
import { beside, pinToViewport, useFollow, zoom } from "./place";

interface SourceChoice {
  ref: string;
  name: string;
  remote: boolean;
  current?: boolean;
  worktree?: string;
}

export default defineComponent({
  name: "WorktreeChip",
  props: {
    port: { type: Object as PropType<AgentPort>, required: true },
    onChanged: { type: Function as PropType<() => void>, required: true },
    onSwitched: { type: Function as PropType<(git: WorkspaceGit) => void> },
    onError: { type: Function as PropType<(error: unknown) => void>, required: true },
    sources: { type: Object as PropType<WorktreeSources>, required: true },
    workspace: { type: null as unknown as PropType<WorkspaceInfo | null>, default: null },
    canCreate: { type: Boolean, required: true },
    selection: { type: null as unknown as PropType<WorktreeSelection | null>, default: null },
    branch: { type: String, required: true },
    busy: { type: Boolean, required: true },
    changeCount: { type: Number, required: true },
    onSelect: { type: Function as PropType<(selection: WorktreeSelection | null) => void>, required: true },
    refresh: { type: Function as PropType<() => Promise<WorktreeSources>>, required: true },
  },
  setup(props) {
    const open = ref(false);
    const query = ref("");
    const sourceError = ref(false);
    const trigger = ref<HTMLButtonElement | null>(null);
    const panel = ref<HTMLDivElement | null>(null);
    const search = ref<HTMLInputElement | null>(null);
    const titleId = useId();
    let returnFocus = false;
    const close = () => { returnFocus = true; open.value = false; query.value = ""; };
    const { locals, moving, reading, loadBranches, switchTo } = useWorkspaceBranches(
      () => props.port,
      () => props.onChanged(),
      (git) => { props.onSwitched?.(git); close(); },
      (error) => props.onError(error),
      () => props.sources.sourceRoot,
    );
    watch([open, moving], ([isOpen, isMoving]) => {
      if (!isOpen && isMoving === null && returnFocus) { returnFocus = false; trigger.value?.focus(); }
    }, { flush: "post" });
    const isolated = computed(() => props.workspace?.current === props.sources.sourceRoot && !!props.workspace.isolated);
    const chosen = computed(() => props.sources.sources.find((source) => source.ref === props.selection?.ref));
    const label = computed(() => chosen.value?.name ?? props.selection?.ref ?? props.branch);
    const creationDescription = t("composer.create_an_isolated_worktree_when");
    const directoryDescription = computed(() => props.selection ? creationDescription : workspaceDirectoryDescription(isolated.value));
    let scope = { port: props.port, root: props.sources.sourceRoot };
    watch([() => props.port, () => props.sources.sourceRoot], ([port, root]) => {
      if (scope.port !== port || scope.root !== root) {
        scope = { port, root };
        returnFocus = false; open.value = false; query.value = "";
      }
    });
    const put = () => {
      if (!trigger.value || !panel.value) return;
      const anchor = trigger.value.getBoundingClientRect();
      const height = panel.value.offsetHeight * zoom();
      pinToViewport(panel.value, anchor.left, beside(anchor, height, innerHeight, "above", 10), 12);
    };
    useFollow(open, put, () => `${query.value}:${reading.value}:${sourceError.value}:${!!props.selection}:${locals.value.length}`, { anchor: trigger, container: panel });
    watch(open, (isOpen, _was, onCleanup) => {
      if (!isOpen) return;
      search.value?.focus();
      const outside = (event: MouseEvent) => {
        if (!panel.value?.contains(event.target as Node) && !trigger.value?.contains(event.target as Node)) close();
      };
      const key = (event: KeyboardEvent) => {
        if (event.key === "Escape") { event.stopPropagation(); close(); }
        if (event.key !== "Tab") return;
        const stops = Array.from(panel.value?.querySelectorAll<HTMLElement>("button:not(:disabled), input, summary") ?? []);
        const edge = event.shiftKey ? stops[0] : stops.at(-1);
        if (document.activeElement === edge) { event.preventDefault(); (event.shiftKey ? stops.at(-1) : stops[0])?.focus(); }
      };
      const stopOutside = listenAction(window, "mousedown", { action: "workspace.configure", listener: outside as EventListener });
      const stopKey = listenAction(window, "keydown", { action: "workspace.configure", listener: key as EventListener }, true);
      const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(put);
      if (panel.value) observer?.observe(panel.value);
      onCleanup(() => { stopOutside(); stopKey(); observer?.disconnect(); });
    }, { flush: "post" });
    watch(() => props.busy, (busy) => { if (busy) open.value = false; });
    const reload = () => {
      sourceError.value = false;
      loadBranches();
      if (props.canCreate) void props.refresh().catch(() => { sourceError.value = true; });
    };
    const show = () => { if (open.value) { close(); return; } open.value = true; query.value = ""; reload(); };
    const selectWorktree = () => {
      query.value = "";
      props.onSelect({ sourceRoot: props.sources.sourceRoot, ref: (props.sources.sources.find((source) => !source.remote && source.name === props.branch) ?? props.sources.sources.find((source) => source.current))?.ref ?? "HEAD" });
    };
    const choices = computed<SourceChoice[]>(() => props.selection
      ? props.sources.sources.map((source) => ({ ...source, worktree: "" }))
      : locals.value.map((source) => ({ ...source, ref: source.name, remote: false })));
    const selected = (source: SourceChoice) => props.selection ? props.selection.ref === source.ref : !!source.current;
    const shown = computed(() => choices.value
      .filter((source) => source.name.toLowerCase().includes(query.value.toLowerCase()))
      .sort((a, b) => Number(selected(b)) - Number(selected(a))));
    const failed = computed(() => props.selection ? sourceError.value : reading.value === "failed");
    const loading = computed(() => !props.selection && (reading.value === "idle" || reading.value === "loading") && locals.value.length === 0);

    const navSources = (event: KeyboardEvent) => {
      const items = Array.from((event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === "ArrowDown" ? at + 1 : event.key === "ArrowUp" ? at - 1 : at;
      if (next !== at) { event.preventDefault(); items[(next + items.length) % items.length]?.focus(); }
      if (event.key === "Home" || event.key === "End") { event.preventDefault(); (event.key === "Home" ? items[0] : items.at(-1))?.focus(); }
    };

    const palette = () => {
      const body: VNodeChild[] = [];
      if (failed.value) {
        body.push(h("div", { class: "workspace-palette-empty", role: "alert" }, [
          h("p", {}, t("composer.could_not_read_branches")),
          h("button", { "data-action": "workspace.source-open", class: "btn", onClick: reload }, t("common.read_again")),
        ]));
      } else if (loading.value) {
        body.push(h("div", { class: "workspace-palette-empty", role: "status" }, t("composer.reading_branches")));
      } else {
        body.push(h("div", {
          class: "worktree-sources",
          role: "listbox",
          "aria-label": t(props.selection ? "common.source_branch" : "composer.local_branches"),
          "data-action": "workspace.source-navigate",
          onKeydown: navSources,
        }, [
          ...[false, true].map((remote) => h("div", { key: String(remote) }, [
            props.selection && shown.value.some((source) => source.remote) && shown.value.some((source) => source.remote === remote) && h("small", { class: "workspace-palette-group" }, t(remote ? "composer.remote_branches" : "composer.local_branches")),
            shown.value.filter((source) => source.remote === remote).map((source) => h("button", {
              key: source.ref,
              "data-action": props.selection ? "workspace.source-select" : "git.branch",
              type: "button",
              role: "option",
              "aria-label": source.name,
              "aria-selected": selected(source),
              disabled: !props.selection && (!!source.worktree || moving.value !== null),
              title: source.worktree ? `${source.name}\n${source.worktree}` : source.name,
              onClick: () => {
                if (props.selection) props.onSelect({ ref: source.ref, sourceRoot: props.sources.sourceRoot });
                else switchTo(source.name);
              },
            }, [
              h(StudioIcon, { name: "branch" }),
              h("span", { class: "workspace-palette-branch" }, [h(WorkspaceBranchName, { value: source.name })]),
              source.worktree
                ? h("small", {}, t("composer.used_by_another_worktree"))
                : selected(source) && h("small", {}, t(props.selection ? "composer.source" : "common.now")),
            ])),
          ])),
          shown.value.length === 0 && h("p", { class: "workspace-palette-empty" }, t("composer.no_matching_branches")),
        ]));
      }
      return h("div", { ref: panel, class: "workspace-palette", role: "dialog", "aria-labelledby": titleId }, [
        h("div", { class: "workspace-palette-heading" }, [
          h("span", { id: titleId }, t("composer.configure_workspace")),
          h("button", { "data-action": "workspace.configure", class: "workspace-palette-close", "aria-label": t("composer.close_workspace_configuration"), onClick: close }, [h(StudioIcon, { name: "close" })]),
        ]),
        h("div", { class: "workspace-palette-identity" }, [
          h(StudioIcon, { name: "folder" }),
          h("strong", { title: props.sources.sourceRoot }, workspaceName(props.sources.sourceRoot)),
          (props.selection || isolated.value) && h("span", { title: directoryDescription.value }, t("common.isolated_worktree")),
        ]),
        props.canCreate && h("div", { class: "worktree-modes" }, [
          h("button", { "data-action": "workspace.worktree-toggle", title: workspaceDirectoryDescription(false), "aria-pressed": !props.selection, disabled: props.busy || moving.value !== null, onClick: () => { props.onSelect(null); query.value = ""; } }, t("composer.current_folder")),
          h("button", { "data-action": "workspace.worktree-toggle", title: creationDescription, "aria-pressed": !!props.selection, disabled: props.busy || moving.value !== null || !props.sources.available, onClick: selectWorktree }, t("common.isolated_worktree")),
        ]),
        props.canCreate && h("p", { class: "workspace-palette-desc" }, props.selection ? creationDescription : workspaceDirectoryDescription(false)),
        props.selection && h("div", { class: "workspace-palette-label" }, t("common.source_branch")),
        h("div", { class: "workspace-palette-search" }, [
          h(StudioIcon, { name: "search" }),
          h("input", {
            "data-action-change": "workspace.source-search",
            "data-action-keydown": "workspace.source-navigate",
            ref: search,
            type: "search",
            "aria-label": t("composer.search_branches"),
            placeholder: t(props.selection ? "composer.search_source_branches" : "composer.search_local_branches"),
            value: query.value,
            onInput: (event: Event) => { query.value = (event.target as HTMLInputElement).value; },
            onKeydown: (event: KeyboardEvent) => {
              if (event.key === "ArrowDown") { event.preventDefault(); panel.value?.querySelector<HTMLElement>('[role="option"]:not(:disabled)')?.focus(); }
            },
          }),
        ]),
        ...body,
        h("p", { class: "workspace-palette-note" }, props.selection ? `${t("composer.a_task_branch_will_be")} · ${t("composer.create_from_the_committed_version")}` : t("composer.switches_immediately_other_sessions_in")),
        h("div", { class: "workspace-palette-footer" }, [h(WorkspacePath, { path: props.sources.sourceRoot, onResize: put })]),
      ]);
    };

    return () => h("div", { class: "studio-branch-pop" }, [
      h("button", {
        ref: trigger,
        type: "button",
        class: "mode plain studio-branch worktree-trigger",
        "data-on": props.selection || isolated.value ? "" : undefined,
        "data-pending": moving.value !== null ? "" : undefined,
        "data-action": "workspace.configure",
        "aria-label": t("composer.configure_workspace"),
        title: `${directoryDescription.value}\n${props.selection ? label.value : props.branch}\n${props.sources.sourceRoot}`,
        "aria-haspopup": "dialog",
        "aria-expanded": open.value,
        disabled: props.busy || moving.value !== null,
        onClick: show,
      }, [
        h(StudioIcon, { name: "branch" }),
        h("span", { class: "lb" }, moving.value ? t("composer.switching_to_branch", { branch: moving.value }) : props.selection || isolated.value ? t("composer.isolated_worktree_branch", { branch: compactWorkspaceValue(label.value) }) : compactWorkspaceValue(props.branch)),
        props.changeCount > 0 && h("small", {}, t("composer.n_changes", { n: props.changeCount })),
        h(StudioIcon, { name: "down" }),
      ]),
      open.value && h(Teleport, { to: "body" }, [palette()]),
    ]);
  },
});
</script>
