<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { GitCommitFiles } from "../../port/workspace";

export interface WorkbenchHistoryProps {
  port: AgentPort;
  /** The repository below the workspace; absent is the workspace's own. */
  repo?: string;
  branch?: string;
  /** Moves when the tree, the branch or the agent's writes did, so an open
   *  graph is read again rather than left showing a past tip. */
  stamp?: string;
  /** How many commits the graph asks for; the kernel bounds it itself. */
  limit?: number;
}

/** One commit's files, and one path's diff within it. Both read-only. */
export type CommitRead =
  | { k: "loading" }
  | { k: "ready"; read: GitCommitFiles }
  | { k: "failed"; why: string };

export type CommitFileRead = { path: string; diff?: string; truncated?: boolean; why?: string };

// git prints a file header above the first hunk; the row above says which file
// the diff belongs to, so the header is dropped.
export function fromFirstHunk(diff: string): string {
  return diff.startsWith("@@") ? diff : diff.slice(diff.indexOf("\n@@") + 1) || diff;
}

export const ROW = 28;
const COL = 12;
const PAD = 8;

export const laneX = (lane: number) => PAD + lane * COL;
export const laneColor = (lane: number) => `var(--cat-${(lane % 5) + 1})`;
export const laneWidth = (lanes: number) => PAD * 2 + Math.max(lanes - 1, 0) * COL;

// A line from (x1, y1) to (x2, y2): straight down a lane, a gentle S across lanes.
export function laneLine(x1: number, y1: number, x2: number, y2: number): string {
  return x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}C${x1} ${(y1 + y2) / 2} ${x2} ${(y1 + y2) / 2} ${x2} ${y2}`;
}

/** A commit's age as the board writes it: a bare count and unit, no "ago". */
export function commitAge(at: number, now: number, say: (key: string, vars?: Record<string, string | number>) => string): string {
  const minutes = Math.floor(Math.max(0, now - at * 1000) / 60000);
  if (minutes < 1) return say("workbench.history_now");
  if (minutes < 60) return say("workbench.history_min", { n: minutes });
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? say("workbench.history_hour", { n: hours }) : say("workbench.history_day", { n: Math.floor(hours / 24) });
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { GitHistory } from "../../port/workspace";
import { layoutGraph } from "../../state/gitgraph";
import StudioIcon from "../StudioIcon.vue";
import UnifiedDiff from "../UnifiedDiff.vue";

const props = withDefaults(defineProps<WorkbenchHistoryProps>(), { stamp: "", limit: 100 });

type Read =
  | { k: "idle" }
  | { k: "loading" }
  | { k: "ready"; history: GitHistory }
  | { k: "failed"; why: string };

// The commit graph beside the changes: what the project's history looks like,
// read from git and never changed from here.
const openId = ref("");
const shown = ref(true);
const read = ref<Read>({ k: "idle" });
const seen = ref<string | null>(null);
let latest = 0;

// The opened commit's files, and the one path whose diff was asked for.
const commitRead = ref<CommitRead>({ k: "loading" });
const openedFile = ref("");
const fileRead = ref<CommitFileRead | null>(null);
let fileTicket = 0;

const key = computed(() => `${props.branch ?? ""}\n${props.stamp}`);

const load = async (quiet = false) => {
  const id = ++latest;
  if (!quiet) read.value = { k: "loading" };
  try {
    const history = await props.port.workspaceGitLog(props.limit, props.repo);
    if (id === latest) read.value = { k: "ready", history };
  } catch (e) {
    if (id === latest && !quiet) read.value = { k: "failed", why: reason(e) };
  }
};

// Read when the section is opened, and again when the key moves: a closed
// section costs nothing, and an open one does not go on showing a past tip.
watch([shown, () => read.value.k, key], () => {
  if (!shown.value) return;
  if (read.value.k === "idle") {
    seen.value = key.value;
    void load();
  } else if (seen.value !== key.value) {
    seen.value = key.value;
    void load(true);
  }
}, { immediate: true });

const history = computed(() => (read.value.k === "ready" ? read.value.history : null));
const graph = computed(() => (history.value?.repo ? layoutGraph(history.value.commits) : null));
const lanes = computed(() => graph.value?.lanes ?? 1);
const now = Date.now();

const toggle = (id: string) => {
  if (openId.value === id) {
    openId.value = "";
    return;
  }
  openId.value = id;
  openedFile.value = "";
  fileRead.value = null;
  commitRead.value = { k: "loading" };
  props.port.commitFiles(id, props.repo).then(
    (r) => { if (openId.value === id) commitRead.value = { k: "ready", read: r }; },
    (e) => { if (openId.value === id) commitRead.value = { k: "failed", why: reason(e) }; },
  );
};

const openFile = (path: string) => {
  if (!openId.value) return;
  if (openedFile.value === path) {
    openedFile.value = "";
    return;
  }
  openedFile.value = path;
  fileRead.value = { path };
  const id = openId.value;
  const ticket = ++fileTicket;
  props.port.commitDiff(id, path, props.repo).then(
    (d) => { if (ticket === fileTicket) fileRead.value = { path, diff: d.diff, truncated: d.truncated }; },
    (e) => { if (ticket === fileTicket) fileRead.value = { path, why: reason(e) }; },
  );
};

// The template reads these; a plain <script> block's bindings are not in scope there.
const row = ROW;
const x = laneX;
const color = laneColor;
const width = () => laneWidth(lanes.value);
const line = laneLine;
const mid = ROW / 2;
const age = commitAge;
const from = fromFirstHunk;
const say = t;
</script>

<template>
  <details class="workbench-history" :open="shown" @toggle="shown = ($event.currentTarget as HTMLDetailsElement).open">
    <summary>
      <StudioIcon name="down" class="gh-fold" />
      <span>{{ t("workbench.commit_history") }}</span>
      <small v-if="branch" class="gh-branch">{{ branch }}</small>
    </summary>
    <p v-if="read.k === 'loading'" class="gh-note" role="status">{{ t("common.loading") }}</p>
    <p v-if="read.k === 'failed'" class="gh-note refine-error" role="alert">
      {{ read.why }}
      <button type="button" class="btn sm" data-action="workbench.history-reload" @click="void load()">{{ t("common.read_again") }}</button>
    </p>
    <p v-if="history && !history.repo" class="gh-note">{{ t("workbench.changes_and_history_need_git") }}</p>
    <p v-if="graph && graph.rows.length === 0" class="gh-note">{{ t("workbench.no_commits_yet") }}</p>
    <ol v-if="graph && graph.rows.length > 0" class="gh-list">
      <li v-for="entry in graph.rows" :key="entry.commit.id" class="gh-item">
        <button
          type="button"
          class="gh-row"
          data-action="workbench.commit"
          :data-target="entry.commit.id"
          :aria-expanded="openId === entry.commit.id"
          :title="`${entry.commit.id.slice(0, 7)}  ${entry.commit.subject}`"
          @click="toggle(entry.commit.id)"
        >
          <svg v-if="lanes > 1" class="gh-lanes" :width="width()" :height="row" :viewBox="`0 0 ${width()} ${row}`" aria-hidden="true">
            <path v-for="lane in entry.through" :key="`t${lane}`" :d="line(x(lane), 0, x(lane), row)" :stroke="color(lane)" />
            <path v-for="lane in entry.from" :key="`f${lane}`" :d="line(x(lane), 0, x(entry.lane), mid)" :stroke="color(lane)" />
            <path v-for="(lane, n) in entry.to" :key="`o${n}`" :d="line(x(entry.lane), mid, x(lane), row)" :stroke="color(lane)" />
            <circle :cx="x(entry.lane)" :cy="mid" r="4" :fill="color(entry.lane)" :data-merge="entry.commit.parents.length > 1 ? '' : undefined" />
          </svg>
          <code class="gh-hash">{{ entry.commit.id.slice(0, 7) }}</code>
          <span class="gh-subject">{{ entry.commit.subject }}</span>
          <em v-for="ref in entry.commit.refs" :key="ref" class="gh-ref" :data-kind="ref.startsWith('tag: ') ? 'tag' : 'branch'">{{ ref }}</em>
          <small class="gh-meta" :title="entry.commit.author">{{ age(entry.commit.at, now, say) }}</small>
        </button>
        <div v-if="openId === entry.commit.id" class="gh-detail">
          <p v-if="commitRead.k === 'loading'" class="gh-note" role="status">{{ t("common.loading") }}</p>
          <p v-else-if="commitRead.k === 'failed'" class="gh-note refine-error" role="alert">{{ commitRead.why }}</p>
          <template v-else>
            <p v-if="commitRead.read.files.length === 0" class="gh-note">{{ t("workbench.this_commit_changed_no_files") }}</p>
            <ul class="gh-files">
              <li v-for="file in commitRead.read.files" :key="file.path">
                <button
                  type="button"
                  class="gh-file"
                  data-action="workbench.commit-file"
                  :data-target="file.path"
                  :aria-expanded="openedFile === file.path"
                  :title="file.oldPath ? `${file.oldPath} → ${file.path}` : file.path"
                  @click="openFile(file.path)"
                >
                  <em :data-status="file.status">{{ file.status }}</em>
                  <span>{{ file.path.slice(file.path.lastIndexOf("/") + 1) }}</span>
                  <i v-if="file.insertions !== undefined" data-io="up">+{{ file.insertions }}</i>
                  <i v-if="file.deletions !== undefined" data-io="down">-{{ file.deletions }}</i>
                </button>
                <div v-if="openedFile === file.path" class="gh-diff">
                  <p v-if="fileRead?.why" class="refine-error" role="alert">{{ fileRead.why }}</p>
                  <p v-else-if="fileRead?.diff === undefined" class="gh-note" role="status">{{ t("common.loading") }}</p>
                  <p v-else-if="from(fileRead.diff).trim() === ''" class="gh-note">{{ t("workbench.there_is_no_text_change") }}</p>
                  <UnifiedDiff v-else :diff="from(fileRead.diff)" :label="file.path" :truncated="!!fileRead.truncated" :target="file.path" />
                </div>
              </li>
            </ul>
            <p v-if="commitRead.read.truncated" class="gh-more">{{ t("workbench.only_the_first_files_are") }}</p>
          </template>
        </div>
      </li>
      <li v-if="history?.truncated" class="gh-more">{{ t("workbench.only_the_most_recent_commits") }}</li>
    </ol>
  </details>
</template>
