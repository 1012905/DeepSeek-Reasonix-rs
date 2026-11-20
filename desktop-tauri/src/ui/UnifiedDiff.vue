<script lang="ts">
/** One line of a unified diff with the line it stands on before and after; a
 *  hunk header carries neither. */
export interface DiffLine { old?: number; now?: number; mark: " " | "+" | "-" | "@"; text: string }

export function diffLines(diff: string): DiffLine[] {
  const out: DiffLine[] = [];
  let old = 0;
  let now = 0;
  const lines = diff.replace(/\n$/, "").split("\n");
  for (const line of lines) {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      old = Number(hunk[1]);
      now = Number(hunk[2]);
      out.push({ mark: "@", text: line });
    } else if (!out.length || line.startsWith("\\")) {
      // File headers come before the first hunk; inside one, a removed line may
      // itself begin with "--", so only git's "\ No newline" note is skipped.
      continue;
    } else if (line.startsWith("+")) {
      out.push({ now: now++, mark: "+", text: line.slice(1) });
    } else if (line.startsWith("-")) {
      out.push({ old: old++, mark: "-", text: line.slice(1) });
    } else {
      out.push({ old: old++, now: now++, mark: " ", text: line.slice(1) });
    }
  }
  return out;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";

const props = withDefaults(defineProps<{ diff: string; label: string; truncated?: boolean; onLine?: () => void; target?: string }>(), { truncated: false });

const lines = computed(() => diffLines(props.diff));
</script>

<template>
  <p v-if="lines.length === 0" class="wb-change-wait">{{ t("workbench.no_changes") }}</p>
  <div v-else class="wb-diff" role="table" :aria-label="label">
    <div v-for="(l, i) in lines" :key="i" class="wb-diff-line" :data-mark="l.mark" role="row">
      <button type="button" class="wb-diff-n" data-action="workbench.change-open" :data-target="target" tabindex="-1" aria-hidden="true" @click="onLine">{{ l.old ?? "" }}</button>
      <button type="button" class="wb-diff-n" data-action="workbench.change-open" :data-target="target" tabindex="-1" aria-hidden="true" @click="onLine">{{ l.now ?? "" }}</button>
      <span class="wb-diff-mark">{{ l.mark === "@" ? "" : l.mark.trim() }}</span>
      <span class="wb-diff-text">{{ l.text }}</span>
    </div>
    <p v-if="truncated" class="wb-change-wait">{{ t("workbench.diff_cut_short") }}</p>
  </div>
</template>