<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { GitRepoRow, WorkspaceChange } from "../../port/workspace";

export interface WorkbenchGitProps {
  port: AgentPort;
  workspaceRoot?: string;
  /** The workspace's own repository's changes, kept current by the panel. A
   *  folder of projects has none of its own: each repository reads its own. */
  changes: WorkspaceChange[];
  repos: GitRepoRow[];
  /** Why the repositories could not be listed, in words, and the way to ask again. */
  failure?: string;
  onRetry?: () => void;
  /** Moves when the agent has written, so a repository the panel does not watch is read again. */
  wrote: number;
  /** The workspace-relative path whose diff is on screen, "" when none is. */
  current: string;
  onOpen: (path: string) => void;
  onChanged?: () => void;
  remote: boolean;
  onOpenPage?: () => void;
  /** Set on the copy this component draws for one repository of a folder of
   *  them: the section itself, with no list above it. */
  single?: GitRepoRow;
}

/** A repository's folder joined to a path inside it, as the panel spells it. */
export function joinRepo(repo: string, path: string): string {
  return repo ? `${repo}/${path}` : path;
}

/** A panel path as the section of one repository spells it: inside that
 *  repository, or "" when it is somewhere else (or one of several). */
export function insideRepo(repo: string, path: string): string {
  if (!repo) return path;
  return path.startsWith(repo + "/") ? path.slice(repo.length + 1) : "";
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { useCommitCard } from "../CommitCard";
import { usePushCard } from "../GitPushCard";
import { useRepoChanges } from "../useRepoChanges";
import StudioIcon from "../StudioIcon.vue";
import WorkbenchChanges from "./WorkbenchChanges.vue";
import WorkbenchHistory from "./WorkbenchHistory.vue";

const props = withDefaults(defineProps<WorkbenchGitProps>(), { failure: "" });

// The Git panel: for each repository of the workspace, what changed as a tree to
// stage, discard and commit, the push of the branch, and its history. A folder of
// projects draws one section per repository, each a copy of this component that
// acts on its own repository alone; the section's repository is fixed for the
// life of its copy, which is what lets the commit and push cards hold it.
const several = computed(() => !props.single && (props.repos.length > 1 || props.repos[0]?.dir !== ""));
// One repository is drawn as itself: the workspace's own by default, or the one
// this copy was given. Several are a list of sections, none of them on its own.
const section = computed<GitRepoRow | null>(() => props.single ?? (several.value ? null : (props.repos[0] ?? null)));

const open = ref<Record<string, boolean>>({});
const opened = (dir: string) => open.value[dir] !== false;
const toggle = (dir: string) => { open.value = { ...open.value, [dir]: !opened(dir) }; };

// A repository below the workspace reads its own changes while its section is
// open; the workspace's own is read by the panel and handed in.
const own = props.single && props.single.dir ? useRepoChanges(() => props.port, () => props.single!.dir, () => opened(props.single!.dir), () => props.wrote) : null;

const changes = computed(() => (props.single && props.single.dir ? own?.changes.value ?? [] : props.changes));
const repo = computed(() => (props.single && props.single.dir ? props.single.dir : undefined));

const refresh = () => {
  props.onChanged?.();
  own?.reload();
};
const commit = useCommitCard(props.port, refresh, repo.value);
const push = usePushCard(props.port, repo.value, refresh);
</script>

<template>
  <p v-if="failure" class="refine-error workbench-stage-error" role="alert">
    {{ failure }}
    <button v-if="onRetry" type="button" class="btn sm" data-action="workbench.repos-reload" @click="onRetry">{{ t("common.read_again") }}</button>
  </p>

  <div v-if="several" class="workbench-git" :data-repos="repos.length">
    <section v-for="row in repos" :key="row.dir" class="workbench-repo" :data-repo="row.dir">
      <button
        type="button"
        class="workbench-repo-head"
        data-action="workbench.repo"
        :data-target="row.dir"
        :aria-expanded="opened(row.dir)"
        @click="toggle(row.dir)"
      >
        <StudioIcon :name="opened(row.dir) ? 'down' : 'chevron'" />
        <b>{{ row.name || row.dir }}</b>
        <StudioIcon name="branch" />
        <span>{{ row.branch }}</span>
        <small />
      </button>
      <WorkbenchGit
        v-if="opened(row.dir)"
        :port="port"
        :workspace-root="workspaceRoot"
        :changes="[]"
        :repos="[row]"
        :wrote="wrote"
        :current="current"
        :on-open="onOpen"
        :on-changed="onChanged"
        :remote="remote"
        :on-open-page="onOpenPage"
        :single="row"
      />
    </section>
  </div>

  <template v-else-if="section">
    <WorkbenchChanges
      :port="port"
      :workspace-root="workspaceRoot"
      :repo="repo"
      :changes="changes"
      :current="insideRepo(section.dir, current)"
      :on-open="(path: string) => onOpen(joinRepo(section!.dir, path))"
      :on-changed="remote ? undefined : refresh"
      :on-open-page="onOpenPage"
    >
      <template v-if="!remote" #commit-button><component :is="commit.button.value" /></template>
      <template v-if="!remote" #actions><component :is="push.button.value" /></template>
      <template v-if="!remote" #commit-card><component :is="commit.card.value" /></template>
      <template v-if="!remote" #below><component :is="push.card.value" /></template>
    </WorkbenchChanges>
    <WorkbenchHistory v-if="!remote" :port="port" :repo="repo" :branch="section.branch" :stamp="`${wrote}:${changes.length}`" />
  </template>
</template>
