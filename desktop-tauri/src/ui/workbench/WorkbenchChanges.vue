<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { StageRequest, WorkspaceChange } from "../../port/port";
import type { ChangeNode } from "../../state/gittree";

export interface WorkbenchChangesProps {
  port: AgentPort;
  workspaceRoot?: string;
  changes: WorkspaceChange[];
  /** The repository below the workspace this list belongs to; absent is the workspace's own. */
  repo?: string;
  /** The path whose diff is on screen, "" when none is. */
  current: string;
  onOpen: (path: string) => void;
  /** Asked for when staging or discarding changed the tree. Absent means this
   *  surface cannot refresh the list, so it offers neither. */
  onChanged?: () => void;
  /** Opens the changes page from this section's title. */
  onOpenPage?: () => void;
}

/** One drawn row: a folder or a file, at its depth in the change tree. */
export interface ChangeRow {
  node: ChangeNode;
  depth: number;
}

// A discard goes back to the last commit, so only a file that commit holds and
// that is modified or deleted can be offered it.
export function discardable(c: WorkspaceChange): boolean {
  return !c.oldPath && /^[MDT]+$/.test(c.status || "M");
}

// The rows drawn at once. A workspace can carry a hundred thousand changes and
// every row is DOM the page holds for as long as the panel is mounted.
export const ROW_PAGE = 500;

/** The tree as rows, cut at the page limit and with closed folders left out.
 *  Depth-first so a folder's own row comes before what is under it. */
export function rowsOf(tree: ChangeNode[], shut: Set<string>, limit: number): ChangeRow[] {
  const out: ChangeRow[] = [];
  const walk = (nodes: ChangeNode[], depth: number) => {
    for (const node of nodes) {
      if (out.length >= limit) return;
      out.push({ node, depth });
      if (node.kind === "folder" && !shut.has(node.path)) walk(node.children, depth + 1);
    }
  };
  walk(tree, 0);
  return out;
}

export function withRepo(repo: string | undefined, req: StageRequest): StageRequest {
  return repo ? { ...req, repo } : req;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import { buildTree, stagedState } from "../../state/gittree";
import { workspaceDrag } from "../filedrag";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<WorkbenchChangesProps>();

// The module-scope helpers reach the template through these: a plain <script>
// block's bindings are not exposed to the template, only to the setup scope.
const canDiscard = discardable;
const page = ROW_PAGE;

// What a commit records is the index, so this section stages, unstages and
// throws changes away; the commit and push cards are drawn by the caller, which
// owns the sections above this one.
const folded = ref(false);
const busy = ref(false);
const failure = ref("");
const shut = ref<Set<string>>(new Set());
const asking = ref("");
const limit = ref(ROW_PAGE);

const tree = computed(() => buildTree(props.changes));
const rows = computed(() => rowsOf(tree.value, shut.value, limit.value));
const filesDrawn = computed(() => rows.value.filter((r) => r.node.kind === "file").length);
const settled = computed(() => props.changes.length > 0 && props.changes.every((c) => c.staged && !c.unstaged));

const run = async (work: () => Promise<void>) => {
  busy.value = true;
  failure.value = "";
  try {
    await work();
    props.onChanged?.();
  } catch (e) {
    failure.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const stage = (req: StageRequest) => run(() => props.port.stageChanges(withRepo(props.repo, req)));
const discard = (path: string) =>
  run(async () => {
    await props.port.discardChange(props.repo ? { repo: props.repo, path } : { path });
    asking.value = "";
  });

const fold = (path: string) => {
  const next = new Set(shut.value);
  if (!next.delete(path)) next.add(path);
  shut.value = next;
};

const dragOf = (path: string) => workspaceDrag(props.workspaceRoot, props.repo ? `${props.repo}/${path}` : path);
const inset = (depth: number) => ({ paddingInlineStart: `${6 + depth * 12}px` });
</script>

<template>
  <div class="workbench-changes">
    <div class="workbench-explorer-head">
      <button
        type="button"
        class="workbench-section-fold"
        data-action="workbench.section-fold"
        data-target="changes"
        :aria-expanded="!folded"
        :aria-label="t(folded ? 'workbench.expand_section' : 'workbench.collapse_section', { name: t('common.changes') })"
        @click="folded = !folded"
      >
        <StudioIcon :name="folded ? 'chevron' : 'down'" />
      </button>
      <button v-if="onOpenPage" type="button" class="workbench-section-title" data-action="workbench.changes-page" @click="onOpenPage">
        {{ t("common.changes") }}
      </button>
      <span v-else>{{ t("common.changes") }}</span>
      <small>{{ t("common.n_files", { n: changes.length }) }}</small>
      <button
        v-if="onChanged"
        type="button"
        class="btn sm workbench-stage-all"
        data-action="workbench.stage-all"
        :disabled="busy"
        @click="void stage({ all: true, unstage: settled })"
      >
        {{ t(settled ? "workbench.unstage_all" : "workbench.stage_all") }}
      </button>
      <slot v-if="onChanged" name="commit-button" />
      <slot v-if="onChanged" name="actions" />
    </div>
    <slot name="commit-card" />
    <slot name="below" />
    <p v-if="failure" class="refine-error workbench-stage-error" role="alert">{{ failure }}</p>
    <template v-if="!folded">
      <template v-for="row in rows" :key="row.node.kind === 'folder' ? `d:${row.node.path}` : `c:${row.node.path}`">
        <div v-if="row.node.kind === 'folder'" role="group" :aria-label="row.node.path">
          <div class="workbench-change workbench-change-folder" :data-staged="stagedState(row.node.files) === true ? '' : undefined">
            <button
              v-if="onChanged"
              type="button"
              role="checkbox"
              class="workbench-stage"
              data-action="workbench.stage-folder"
              :data-target="row.node.path"
              :aria-checked="stagedState(row.node.files)"
              :aria-label="t('workbench.stage_path', { path: row.node.path + '/' })"
              :disabled="busy"
              @click="void stage({ paths: row.node.files.map((f) => f.path), unstage: stagedState(row.node.files) === true })"
            >
              <StudioIcon name="check" />
            </button>
            <button
              type="button"
              class="workbench-change-row workbench-folder-row"
              :style="inset(row.depth)"
              data-action="workbench.change-folder"
              :data-target="row.node.path"
              v-bind="dragOf(row.node.path)"
              :aria-expanded="!shut.has(row.node.path)"
              @click="fold(row.node.path)"
            >
              <StudioIcon :name="shut.has(row.node.path) ? 'chevron' : 'down'" />
              <span>{{ row.node.name }}</span>
              <small>{{ row.node.files.length }}</small>
              <i v-if="row.node.insertions > 0" data-io="up">+{{ row.node.insertions }}</i>
              <i v-if="row.node.deletions > 0" data-io="down">-{{ row.node.deletions }}</i>
            </button>
          </div>
        </div>
        <div v-else>
          <div class="workbench-change" :data-staged="row.node.change.staged ? '' : undefined">
            <button
              v-if="onChanged"
              type="button"
              role="checkbox"
              class="workbench-stage"
              data-action="workbench.stage"
              :data-target="row.node.path"
              :aria-checked="stagedState([row.node.change])"
              :aria-label="t('workbench.stage_path', { path: row.node.path })"
              :disabled="busy"
              @click="void stage({ paths: [row.node.path], unstage: row.node.change.staged === true && !row.node.change.unstaged })"
            >
              <StudioIcon name="check" />
            </button>
            <button
              class="workbench-change-row"
              :style="inset(row.depth)"
              data-action="workbench.diff"
              :data-target="row.node.path"
              :title="row.node.path"
              v-bind="dragOf(row.node.path)"
              :aria-current="current === row.node.path ? 'page' : undefined"
              @click="onOpen(row.node.path)"
            >
              <em :data-status="row.node.change.status">{{ row.node.change.status || "M" }}</em>
              <span>{{ row.node.name }}</span>
              <!-- Absent counts print nothing: a binary file did not change by
                   zero lines, git simply did not count it. -->
              <i v-if="row.node.change.insertions !== undefined" data-io="up">+{{ row.node.change.insertions }}</i>
              <i v-if="row.node.change.deletions !== undefined" data-io="down">-{{ row.node.change.deletions }}</i>
            </button>
            <button
              v-if="onChanged && canDiscard(row.node.change)"
              type="button"
              class="workbench-discard"
              data-action="workbench.discard"
              :data-target="row.node.path"
              :aria-label="t('workbench.discard_the_change_to_path', { path: row.node.path })"
              :title="t('workbench.discard_this_files_change')"
              :disabled="busy"
              @click="asking = row.node.path"
            >
              <StudioIcon name="rewind" />
            </button>
          </div>
          <div v-if="asking === row.node.path" class="workbench-discard-ask" role="group" :aria-label="t('workbench.confirm_discard')">
            <p>{{ t("workbench.discard_the_change_to_path_2", { path: row.node.path }) }}</p>
            <p v-if="row.node.change.staged && row.node.change.unstaged">{{ t("workbench.both_its_staged_part_and") }}</p>
            <div>
              <button type="button" class="btn sm" data-danger="" data-action="workbench.discard-confirm" :data-target="row.node.path" :disabled="busy" @click="void discard(row.node.path)">
                {{ t("workbench.discard") }}
              </button>
              <button type="button" class="btn sm" data-weak="" data-action="workbench.discard-cancel" :disabled="busy" @click="asking = ''">
                {{ t("common.cancel") }}
              </button>
            </div>
          </div>
        </div>
      </template>
      <div v-if="rows.length >= limit && changes.length > filesDrawn" class="workbench-change-more">
        <span>{{ t("workbench.n_more_not_shown", { n: changes.length - filesDrawn }) }}</span>
        <button type="button" class="btn sm" data-weak="" data-action="workbench.more-changes" @click="limit += page">
          {{ t("workbench.show_n_more", { n: page }) }}
        </button>
      </div>
    </template>
  </div>
</template>
