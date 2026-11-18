<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import type { Item } from "../state/session";
import type { ExtensionSurface } from "../port/wire";
import type { AgentPort, RewindPlan, RewindResult } from "../port/port";
import ToolCard from "./cards/ToolCard.vue";
import StudioIcon from "./StudioIcon.vue";

/** The calls a reader brought to the end, each drawn from the transcript's own
 *  record of it. The original card stays where it happened. */
const props = defineProps<{
  items: Item[];
  ids: string[];
  onDismiss: (id: string) => void;
  takeovers: Record<string, ExtensionSurface>;
  onExtInvoke: (name: string) => void;
  onPrepareFileRevert: (path: string) => Promise<RewindPlan>;
  onCommitFileRevert: (planId: string, resolution?: string) => Promise<RewindResult>;
  port?: Pick<AgentPort, "stopToolCall">;
}>();

const drawn = computed(() => props.ids.flatMap((id) => {
  const it = props.items.find((x) => x.t === "tool" && x.tool.id === id);
  return it && it.t === "tool" ? [it] : [];
}));
</script>

<template>
  <section v-if="drawn.length > 0" class="brought" :aria-label="t('chat.brought_to_the_end')">
    <div v-for="it in drawn" :key="it.tool.id" class="brought-item">
      <div class="brought-bar">
        <span>{{ t("chat.brought_to_the_end") }}</span>
        <button type="button" class="brought-close" data-action="tool.unbring" :aria-label="t('common.remove')" :title="t('common.remove')" @click="onDismiss(it.tool.id!)">
          <StudioIcon name="close" />
        </button>
      </div>
      <div class="activity-body brought-line">
        <ToolCard
          :tool="it.tool"
          :running="it.running"
          :children="it.children"
          :takeover="it.tool.id ? takeovers[`tool:${it.tool.id}`] : undefined"
          :on-ext-invoke="onExtInvoke"
          :on-prepare-file-revert="onPrepareFileRevert"
          :on-commit-file-revert="onCommitFileRevert"
          pinned
        />
      </div>
    </div>
  </section>
</template>