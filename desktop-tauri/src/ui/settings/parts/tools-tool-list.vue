<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, ToolSetting, ToolSettings } from "../../../port/port";
import { toolBlurb } from "../../toolBlurbs";
import Switch from "../../Switch.vue";

// What needs a tool, by the kernel's feature id; an id this build does not know
// is shown as sent rather than guessed at.
const FEATURE: Record<string, string> = {
  goal_mode: "ext.goal_mode",
  deferred_mcp_tools: "ext.loading_deferred_mcp_tools",
  plan_steps: "ext.plan_step_tracking",
  turn_exit: "ext.ending_a_turn_the_host",
};

const props = defineProps<{ port: AgentPort }>();

const state = ref<ToolSettings | null>(null);
const busy = ref("");
const error = ref("");

// A brokered pane's port answers for its own machine, so the list is re-read
// when the port changes rather than carried across.
watch(() => props.port, (port) => {
  let live = true;
  state.value = null;
  error.value = "";
  port.toolSettings()
    .then((value) => { if (live) state.value = value; })
    .catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

const flip = async (tool: ToolSetting) => {
  busy.value = tool.name;
  error.value = "";
  try {
    state.value = await props.port.saveToolDisabled(tool.name, !tool.disabled);
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = "";
  }
};

// The kernel's description is written for the model. A tool this build has a
// line for is named by what it does; only an unknown one falls back to that text.
const says = (tool: ToolSetting): string => {
  if (tool.requiredBy) {
    const key = FEATURE[tool.requiredBy];
    return t("ext.feature_needs_it_so_it", { feature: key ? t(key) : tool.requiredBy });
  }
  if (tool.byProject) return t("ext.this_projects_config_file_turned");
  return toolBlurb(tool.name) ?? tool.description;
};
</script>

<template>
  <div v-if="!state" class="empty">{{ t("ext.could_not_read_the_tool") }}</div>
  <div v-else class="box">
    <div v-for="tool in state.tools" :key="tool.name" class="lrow">
      <span class="tx">
        <span class="lb">{{ tool.name }}</span>
        <span class="ds">{{ says(tool) }}</span>
      </span>
      <Switch
        v-if="!tool.requiredBy && !tool.byProject"
        data-action="tool-list.toggle"
        :data-target="tool.name"
        :on="!tool.disabled"
        :busy="busy === tool.name"
        :label="tool.name"
        :on-click="() => void flip(tool)"
      />
    </div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
