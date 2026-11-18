<script lang="ts">
type Row = { sign: " " | "+" | "-"; text: string; no: number | null } | { skipped: number };

// A unified diff carries its line numbers in the @@ header, not in row order;
// numbering by row index put a change at line 205 on the card as 1, 2, 3.
export function parseDiff(diff: string): Row[] {
  const out: Row[] = [];
  let oldNo = 0;
  let newNo = 0;
  let lastNew = 0;
  let sawHunk = false;
  const lines = diff.replace(/\n+$/, "").split("\n");
  // A `git show` / `git log -p` result leads with a commit header; the rows are
  // the diff's, so start at the first file.
  const from = /^commit \S/.test(lines[0] ?? "")
    ? Math.max(0, lines.findIndex((l) => l.startsWith("diff --git ") || l.startsWith("--- ")))
    : 0;
  for (const raw of lines.slice(from)) {
    const at = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
    if (at) {
      const start = Number(at[2]);
      if (sawHunk && start > lastNew + 1) out.push({ skipped: start - lastNew - 1 });
      oldNo = Number(at[1]);
      newNo = start;
      sawHunk = true;
      continue;
    }
    if (/^(diff |index |--- |\+\+\+ |new file|deleted file|similarity index|rename )/.test(raw)) continue;
    const c = raw[0];
    const sign: " " | "+" | "-" = c === "+" || c === "-" ? c : " ";
    const text = sign === " " ? raw : raw.slice(1);
    if (!sawHunk) {
      // A bare fragment without @@ has no true line number to give.
      out.push({ sign, text, no: null });
      continue;
    }
    if (sign === "+") {
      out.push({ sign, text, no: newNo });
      lastNew = newNo++;
    } else if (sign === "-") {
      // A removed line is absent from the new file, so its number column stays
      // empty: the column means where a jump would land, and mixing the two
      // files makes the numbers indistinguishable.
      out.push({ sign, text, no: null });
      oldNo++;
    } else {
      out.push({ sign, text, no: newNo });
      lastNew = newNo++;
      oldNo++;
    }
  }
  return out;
}

/** The file a diff names, for a card with no tool args: the first file's
 *  "+++ b/…", falling back to "--- a/…" for a deletion. */
export function pathFromDiff(diff: string): string | undefined {
  let oldPath: string | undefined;
  for (const line of diff.split("\n")) {
    const plus = /^\+\+\+ (?:b\/)?(\S+)/.exec(line);
    if (plus && plus[1] !== "/dev/null") return plus[1];
    const minus = /^--- (?:a\/)?(\S+)/.exec(line);
    if (minus && minus[1] !== "/dev/null") oldPath ??= minus[1];
  }
  return oldPath;
}

/** One file's worth of a diff: the name it carries, its +/- tally, and rows. */
export type Section = { path?: string; added: number; removed: number; rows: Row[] };

/** Cuts a diff into per-file sections by the same rule the CLI uses: a
 *  git-style diff splits on "diff --git ", anything else on "--- "/"+++ ". */
export function splitDiffSections(diff: string): string[] {
  const lines = diff.replace(/\n+$/, "").split("\n");
  const gitStyle = lines.some((l) => l.startsWith("diff --git "));
  const sections: string[] = [];
  let cur: string[] = [];
  lines.forEach((ln, i) => {
    if (cur.length > 0 && diffSectionStart(lines, i, gitStyle)) {
      sections.push(cur.join("\n"));
      cur = [];
    }
    cur.push(ln);
  });
  if (cur.length > 0) sections.push(cur.join("\n"));
  return sections;
}

function diffSectionStart(lines: string[], i: number, gitStyle: boolean): boolean {
  if (gitStyle) return lines[i].startsWith("diff --git ");
  return lines[i].startsWith("--- ") && i + 1 < lines.length && lines[i + 1].startsWith("+++ ");
}

function sectionPath(section: string): string | undefined {
  let oldPath: string | undefined;
  for (const line of section.split("\n")) {
    const plus = /^\+\+\+ (?:b\/)?(\S+)/.exec(line);
    if (plus && plus[1] !== "/dev/null") return plus[1];
    const minus = /^--- (?:a\/)?(\S+)/.exec(line);
    if (minus && minus[1] !== "/dev/null") oldPath ??= minus[1];
  }
  return oldPath;
}

function hasDiffBody(section: string): boolean {
  return section.split("\n").some((l) => l.startsWith("@@ ") || l.startsWith("+") || l.startsWith("-"));
}

function countSection(section: string): { added: number; removed: number } {
  const lines = section.split("\n");
  if (lines.length >= 2 && lines[0].startsWith("--- ") && lines[1].startsWith("+++ ")) lines.splice(0, 2);
  let added = 0;
  let removed = 0;
  for (const ln of lines) {
    if (ln.startsWith("+++ ") || ln.startsWith("--- ")) continue;
    if (ln.startsWith("+")) added++;
    else if (ln.startsWith("-")) removed++;
  }
  return { added, removed };
}

/** Splits a diff into files, each with its own rows and header facts. A section
 *  that is only a commit preamble is dropped. */
export function parseDiffSections(diff: string): Section[] {
  const out: Section[] = [];
  for (const sec of splitDiffSections(diff)) {
    const path = sectionPath(sec);
    if (path === undefined && !hasDiffBody(sec)) continue;
    out.push({ path, ...countSection(sec), rows: parseDiff(sec) });
  }
  return out;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import StudioIcon from "../StudioIcon.vue";
import type { RewindPlan, RewindResult } from "../../port/session";

const props = withDefaults(defineProps<{
  diff: string;
  path?: string;
  /** The surface around this diff already names the file. */
  named?: boolean;
  onPrepare?: (path: string) => Promise<RewindPlan>;
  onCommit?: (planId: string, resolution?: string) => Promise<RewindResult>;
}>(), { path: undefined, named: false, onPrepare: undefined, onCommit: undefined });

interface DrawnRow { skipped?: number; sign?: "+" | "-" | " "; text?: string; no?: number | null }

const sections = computed(() => parseDiffSections(props.diff).map((sec) => ({
  path: sec.path,
  added: sec.added,
  removed: sec.removed,
  rows: sec.rows.map((r): DrawnRow => ("skipped" in r
    ? { skipped: r.skipped }
    : { sign: r.sign, text: r.text, no: r.no })),
})));

// A multi-file diff names each file in its own body header, so the headline
// would only repeat the first; a single-file diff has no body header.
const multi = computed(() => sections.value.length > 1);
const shown = computed(() => props.path ?? pathFromDiff(props.diff));
const plan = ref<RewindPlan | null>(null);
const busy = ref(false);
const outcome = ref<"" | "reverted" | "kept" | "refused">("");
const failed = ref("");
const revertable = computed(() => Boolean(props.path && props.onPrepare && props.onCommit && !outcome.value));
const clash = computed(() => plan.value?.conflicts?.[0]);

const prepare = async () => {
  const path = props.path;
  const onPrepare = props.onPrepare;
  if (!path || !onPrepare) return;
  busy.value = true;
  failed.value = "";
  try {
    const got = await onPrepare(path);
    // The kernel refuses some files outright and says so on the plan; offering
    // the confirm anyway posts an empty planId and answers "missing planId".
    if (!got.canFiles || !got.planId) {
      outcome.value = "refused";
      failed.value = got.disabledReason || t("chat.this_file_cannot_be_restored");
      return;
    }
    plan.value = got;
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const commit = async (resolution?: string) => {
  const held = plan.value;
  const onCommit = props.onCommit;
  if (!held || !onCommit) return;
  busy.value = true;
  failed.value = "";
  try {
    const result = await onCommit(held.planId, resolution);
    plan.value = null;
    // keep_current is the kernel's deliberate no-op: reporting it as "reverted"
    // tells the reader the opposite of what they chose.
    outcome.value = result.written?.length || result.deleted?.length ? "reverted" : "kept";
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div class="dif">
    <div class="dif-hd">
      <span :title="shown ?? undefined">{{ named ? "" : (shown ?? t("common.changes")) }}</span>
      <span v-if="outcome === 'reverted'" class="ro">{{ t("chat.reverted") }}</span>
      <span v-else-if="outcome === 'kept'" class="ro">{{ t("chat.kept_current") }}</span>
      <button v-else-if="revertable && !plan" class="dif-act" data-action="file-revert.prepare" :disabled="busy" @click="prepare">
        {{ t(busy ? "…" : "chat.revert_this_file") }}
      </button>
      <span v-else class="ro">{{ t("common.read_only") }}</span>
    </div>

    <div v-if="plan" class="dif-ask" :data-clash="clash ? '' : undefined">
      <template v-if="clash">
        <div class="q">{{ t("chat.this_file_has_changed_again") }}</div>
        <div class="row">
          <button class="dif-act" data-action="file-revert.commit" data-value="overwrite_checkpoint" :disabled="busy" @click="commit('overwrite_checkpoint')">
            {{ t("chat.overwrite_with_the_checkpoints") }}
          </button>
          <button class="dif-act ghost" data-action="file-revert.commit" data-value="keep_current" :disabled="busy" @click="commit('keep_current')">
            {{ t("chat.keep_the_current_one") }}
          </button>
          <button class="dif-act ghost" @click="plan = null">{{ t("common.cancel") }}</button>
        </div>
      </template>
      <template v-else>
        <!-- The preimage is the session's first, not this turn's: "before this
             change" would promise a surgical undo and deliver a session-wide one. -->
        <div class="q">{{ t("chat.put_this_file_back_the") }}</div>
        <div class="row">
          <button class="dif-act" data-action="file-revert.commit" :disabled="busy" @click="commit()">
            {{ t(busy ? "common.restoring" : "common.revert") }}
          </button>
          <button class="dif-act ghost" @click="plan = null">{{ t("common.cancel") }}</button>
        </div>
      </template>
    </div>
    <div v-if="failed" class="dif-ask">{{ failed }}</div>

    <!-- Horizontal scrolling lives here rather than on the whole block: the
         line-number column would scroll away with the lines. -->
    <div class="dlwrap">
      <template v-for="(sec, si) in sections" :key="si">
        <div v-if="multi && sec.path !== undefined" class="dfile">
          <span class="p" :title="sec.path">{{ sec.path }}</span>
          <span class="st">
            <span v-if="sec.added > 0" class="add">+{{ sec.added }}</span>
            <span v-if="sec.removed > 0" class="del">-{{ sec.removed }}</span>
          </span>
        </div>
        <template v-for="(l, i) in sec.rows" :key="i">
          <div v-if="l.skipped !== undefined" class="dhunk">
            <span><StudioIcon name="more" /></span>
            <span>{{ t("chat.n_lines_skipped", { n: l.skipped }) }}</span>
          </div>
          <div v-else class="dl" :data-d="l.sign === ' ' ? undefined : l.sign">
            <span class="no">{{ l.no ?? "" }}</span>
            <span class="sg">{{ l.sign }}</span>
            <span class="cd">{{ l.text }}</span>
          </div>
        </template>
      </template>
    </div>
  </div>
</template>
