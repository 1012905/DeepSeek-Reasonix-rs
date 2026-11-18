<script lang="ts">
import type { Change } from "./derive";

// The filename is the part being read; a deep directory must not push it out
// under an end-ellipsis. Keep the last two segments and mark the elision, with
// the whole path on the row's title for when it matters.
function elide(dir: string): string {
  const parts = dir.split("/").filter(Boolean);
  return parts.length <= 2 ? dir : `…/${parts.slice(-2).join("/")}/`;
}

// The badge is the porcelain letter itself, coloured by what it means; the
// word is kept for the row's title, where there is room to spell it.
const MARK: Record<string, string> = { M: "panels.modified", A: "ext.added", "??": "ext.added", D: "panels.deleted", R: "common.renamed" };

// "??" is two characters wide and every other code is one, so the badge takes
// the first letter and untracked gets the single question mark it reads as.
function badge(status: string): string {
  const s = status.trim();
  return s === "??" ? "?" : s.slice(0, 1);
}

// A rail panel is a read on the run, not a full manifest. A session that edits
// thousands of files would otherwise put a row per file in the DOM — at 4000
// turns that panel alone was 28k nodes, an order of magnitude more than the
// whole transcript. The count in the header stays honest about the total.
const SHOWN = 60;

/** What the rail actually lists, which the overview counts must agree with —
 *  two derivations of "how many files changed" drift the moment one of them
 *  learns about git and the other does not. */
export function visible(changes: Change[], tree: WorkspaceChanges | null) {
  // git is the authority when there is one: a path the session touched but the
  // tree no longer reports has been undone — created then removed, or edited
  // then reverted — and listing it keeps a change pending that does not exist.
  const byPath = tree?.repo ? new Map(tree.changes.map((c) => [c.path, c.status])) : null;
  // git filters, it does not supply: a repository is usually dirty for reasons
  // that have nothing to do with this session, and listing all of it buried the
  // agent's own edits under a page of files at +0 −0.
  return changes
    .filter((f) => !byPath || byPath.has(f.path))
    .map((f) => ({ ...f, status: byPath?.get(f.path) ?? "" }));
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import type { WorkspaceChanges } from "../../port/port";
import { splitPath } from "../args";
import { t } from "../../i18n";

const props = defineProps<{
  changes: Change[];
  yolo: boolean;
  tree: WorkspaceChanges | null;
  // Absent when nothing can be opened — a window with no kernel behind it, or
  // a workspace git cannot answer for. The rows then stay what they were: a
  // readout, with no affordance promising something that would not happen.
  open?: string | null;
  onOpen?: (path: string) => void;
}>();

// Nothing here outlives an unchanged tree: the count is zero and the list is
// the sentence "no changes yet". A block whose whole content is that it has
// none is the same standing zero the rest of this rail stopped drawing.
const files = computed(() => visible(props.changes, props.tree));
// The tail is the recent end: early files in a long run have been looked at
// already, and the ones still moving are the ones worth a row.
const shown = computed(() => (files.value.length > SHOWN ? files.value.slice(-SHOWN) : files.value));
const rows = computed(() => shown.value.map((f) => {
  const [dir, name] = splitPath(f.path);
  const mk = badge(f.status);
  const word = MARK[f.status] ?? "";
  const title = [f.path, word && t(word), f.edits > 1 ? t("panels.edited_n_times", { n: f.edits }) : ""]
    .filter(Boolean)
    .join(" · ");
  return { f, dir, name, mk, title };
}));
</script>

<template>
  <div v-if="files.length > 0" class="block" data-b="files">
    <div class="lbl">
      <span>{{ t("panels.worktree_changes") }}</span>
      <span class="c">{{ files.length }}{{ files.length > 0 ? ` · ${t(yolo ? "panels.auto_approved" : "panels.for_review")}` : "" }}</span>
    </div>
    <div class="files">
      <span v-if="shown.length < files.length" class="empty">{{ t("panels.n_earlier_files_not_listed", { n: files.length - shown.length }) }}</span>
      <template v-for="r in rows" :key="r.f.path">
        <button v-if="onOpen" class="file" type="button" :title="r.title"
          :data-on="r.f.path === open ? '' : undefined" :aria-pressed="r.f.path === open"
          @click="onOpen(r.f.path)">
          <em v-if="r.mk" class="st" :data-s="r.mk">{{ r.mk }}</em>
          <span class="p">
            <span v-if="r.dir" class="d">{{ elide(r.dir) }}</span>
            <span class="f">{{ r.name }}</span>
          </span>
          <span class="n">
            <span class="a">+{{ r.f.added }}</span> <span class="r">−{{ r.f.removed }}</span>
          </span>
        </button>
        <div v-else class="file" :title="r.title">
          <em v-if="r.mk" class="st" :data-s="r.mk">{{ r.mk }}</em>
          <span class="p">
            <span v-if="r.dir" class="d">{{ elide(r.dir) }}</span>
            <span class="f">{{ r.name }}</span>
          </span>
          <span class="n">
            <span class="a">+{{ r.f.added }}</span> <span class="r">−{{ r.f.removed }}</span>
          </span>
        </div>
      </template>
    </div>
  </div>
</template>