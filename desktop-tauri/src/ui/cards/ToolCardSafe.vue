<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { StateTool } from "../../state/outputcap";
import type { RewindPlan, RewindResult } from "../../port/port";
import type { ExtensionSurface } from "../../port/wire";
import type { ReadOutput } from "./useFullOutput";
import Boundary from "../Boundary.vue";
import ToolCard from "./ToolCard.vue";

// The first string the call was given: the path or command it acted on.
function target(args: string | undefined): string {
  try {
    const parsed: unknown = JSON.parse(args ?? "");
    if (parsed && typeof parsed === "object") {
      for (const value of Object.values(parsed)) if (typeof value === "string" && value) return value.split("\n")[0].slice(0, 120);
    }
  } catch {
    // arguments that do not parse have no target to name
  }
  return "";
}

/** One call's card, falling back to a single line when drawing it throws, so a
    card that cannot be drawn costs that call its detail and nothing else. */
const props = withDefaults(defineProps<{
  tool: StateTool;
  running: boolean;
  activity?: boolean;
  children?: StateTool[];
  readOutput?: ReadOutput;
  omitted?: number;
  onPrepareFileRevert?: (path: string) => Promise<RewindPlan>;
  onCommitFileRevert?: (planId: string, resolution?: string) => Promise<RewindResult>;
  onStop?: () => void;
  onBring?: () => void;
  pinned?: boolean;
  takeover?: ExtensionSurface;
  onExtInvoke?: (actionId: string) => void;
}>(), {
  activity: false, children: () => [], readOutput: undefined, omitted: 0,
  onPrepareFileRevert: undefined, onCommitFileRevert: undefined, onStop: undefined,
  onBring: undefined, pinned: false, takeover: undefined, onExtInvoke: undefined,
});

const to = computed(() => target(props.tool.args));
</script>

<template>
  <Boundary :retry-key="[tool.args, tool.output, running] as never">
    <ToolCard
      :tool="tool"
      :running="running"
      :activity="activity"
      :children="children"
      :read-output="readOutput"
      :omitted="omitted"
      :on-prepare-file-revert="onPrepareFileRevert"
      :on-commit-file-revert="onCommitFileRevert"
      :on-stop="onStop"
      :on-bring="onBring"
      :pinned="pinned"
      :takeover="takeover"
      :on-ext-invoke="onExtInvoke"
    />
    <template #fallback>
      <div class="toolfb" role="group">
        <span class="toolfb-nm">{{ tool.name }}{{ to ? ` · ${to}` : "" }}</span>
        <span class="srcfb-tag">{{ t("chat.simplified_view") }}</span>
      </div>
    </template>
  </Boundary>
</template>
