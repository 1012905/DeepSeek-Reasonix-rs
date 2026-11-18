<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import {
  FOLD_CHOICES, FOLD_DEFAULTS, setFoldModes, setShowsThinkingPreview,
  type Fold, type FoldMode, type FoldModes,
} from "../../../state/prefs";
import { startsOpen, useFoldModes, useThinkingPreview } from "../../../state/foldpref";
import Group from "../../Group.vue";
import Switch from "../../Switch.vue";

const PARTS: [Fold, string, string][] = [
  ["thinking", "chat.thinking_2", "chat.the_models_reasoning_before_it"],
  ["activity", "chat.activity", "chat.a_run_of_tool_calls"],
  ["steps", "chat.step_details", "chat.each_steps_arguments_output_and"],
  ["output", "chat.long_output", "chat.command_output_and_file_contents"],
  ["compaction", "chat.compaction_brief", "chat.the_brief_the_model_works"],
];

const MODE: Record<FoldMode, string> = {
  folded: "common.collapse",
  live: "chat.open_while_running",
  failed: "chat.open_on_failure",
  changed: "chat.open_on_file_change",
  open: "common.expand",
};

// Long output is clipped rather than folded, so its two answers say so.
const OUTPUT_MODE: Partial<Record<FoldMode, string>> = { folded: "chat.clipped", open: "look.full" };

const PRESETS: [string, string, FoldModes][] = [
  ["compact", "chat.compact", { thinking: "folded", activity: "folded", steps: "failed", output: "folded", compaction: "folded" }],
  ["standard", "look.standard", FOLD_DEFAULTS],
  ["changes", "chat.changes_only", { thinking: "folded", activity: "changed", steps: "changed", output: "folded", compaction: "folded" }],
  ["full", "chat.detailed", { thinking: "open", activity: "open", steps: "open", output: "open", compaction: "open" }],
];

const same = (a: FoldModes, b: FoldModes) => PARTS.every(([k]) => a[k] === b[k]);

const modes = useFoldModes();
const thinkingPreview = useThinkingPreview();
const preset = computed(() => PRESETS.find(([, , p]) => same(p, modes.value))?.[0]);

/** Whether the part starts open, stated for the two moments a reader can be in:
 *  while a turn runs, and after it finished without failing or changing a file.
 *  A block opened or closed by hand keeps that answer instead. */
const openRunning = (kind: Fold) => startsOpen(modes.value[kind], true, false, false);
const openDone = (kind: Fold) => startsOpen(modes.value[kind], false, false, false);
const said = (open: boolean) => (open ? t("common.expand") : t("common.collapse"));

const modeName = (kind: Fold, mode: FoldMode) => t((kind === "output" && OUTPUT_MODE[mode]) || MODE[mode]);
</script>

<template>
  <Group
    id="folding"
    :title="t('chat.transcript_folding')"
    :now="preset ? undefined : t('common.custom')"
    :hint="t('chat.whether_each_part_of_a')"
  >
    <div class="seg" data-text role="radiogroup" :aria-label="t('chat.folding_preset')">
      <button
        v-for="[id, name, p] in PRESETS"
        :key="id"
        role="radio"
        data-action="transcript.fold-preset"
        :data-value="id"
        :aria-checked="preset === id"
        @click="setFoldModes(p)"
      >
        {{ t(name) }}
      </button>
    </div>

    <div class="fold-rows">
      <div v-for="[kind, label, why] in PARTS" :key="kind" class="fold-row">
        <span class="tx">
          <span class="lb">{{ t(label) }}</span>
          <span class="ds">{{ t(why) }}</span>
          <span class="ds">
            {{ t("common.running") }}: {{ said(openRunning(kind)) }} · {{ t("common.done_2") }}: {{ said(openDone(kind)) }}
          </span>
        </span>
        <div class="seg" data-text role="radiogroup" :aria-label="t(label)">
          <button
            v-for="mode in FOLD_CHOICES[kind]"
            :key="mode"
            role="radio"
            data-action="transcript.fold"
            :data-value="`${kind}:${mode}`"
            :aria-checked="modes[kind] === mode"
            @click="setFoldModes({ [kind]: mode })"
          >
            {{ modeName(kind, mode) }}
          </button>
        </div>
      </div>

      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("chat.preview_the_beginning_when_collapsed") }}</span>
          <span class="ds">{{ t("chat.show_one_line_of_opening") }}</span>
        </span>
        <Switch
          data-action="transcript.thinking-preview"
          :on="thinkingPreview"
          :label="t('chat.preview_the_beginning_when_collapsed')"
          :on-click="() => setShowsThinkingPreview(!thinkingPreview)"
        />
      </div>
    </div>
  </Group>
</template>
