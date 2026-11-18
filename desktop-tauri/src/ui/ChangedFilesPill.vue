<script lang="ts">
import type { WorkspaceChange } from "../port/workspace";

export interface ChangeTotals {
  files: number;
  insertions: number;
  deletions: number;
  counted: boolean;
}

export function totalsOf(changes: readonly WorkspaceChange[]): ChangeTotals {
  let insertions = 0;
  let deletions = 0;
  let counted = false;
  for (const c of changes) {
    if (c.insertions !== undefined) {
      insertions += c.insertions;
      counted = true;
    }
    if (c.deletions !== undefined) {
      deletions += c.deletions;
      counted = true;
    }
  }
  return { files: changes.length, insertions, deletions, counted };
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { plural, t } from "../i18n";
import { tokens } from "../i18n/format";

// The workspace's differing files as git counts them, in one place above the
// input box. It reads the same list as the workbench's change section and
// opens it; nothing here is the turn's own edits.
const props = defineProps<{ changes: readonly WorkspaceChange[]; onOpen: () => void }>();

const sum = computed(() => totalsOf(props.changes));
const label = computed(() => plural(sum.value.files, "composer.n_1_file_changed", "composer.n_files_changed", { n: tokens(sum.value.files) }));
const aria = computed(() => sum.value.counted
  ? t("composer.label_up_lines_added_down", { label: label.value, up: sum.value.insertions, down: sum.value.deletions })
  : t("composer.label_open_file_changes", { label: label.value }));
</script>

<template>
  <div v-if="changes.length > 0" class="changedpill-row">
    <button class="changedpill" data-action="changes.open" :aria-label="aria" :title="t('composer.view_file_changes')" @click="onOpen">
      <span>{{ label }}</span>
      <template v-if="sum.counted">
        <i data-io="up">+{{ tokens(sum.insertions) }}</i>
        <i data-io="down">−{{ tokens(sum.deletions) }}</i>
      </template>
    </button>
  </div>
</template>