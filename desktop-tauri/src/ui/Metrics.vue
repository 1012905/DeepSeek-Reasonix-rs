<script lang="ts">
import type { AgentPort, ContextBreakdown, McpEntry, WorkspaceChanges } from "../port/port";
import type { Change } from "./panels/derive";

/** What the rail draws. Presence is still each panel's own contract — a panel
 *  with nothing to say draws nothing — so this decides where something goes,
 *  never whether there is something. */
export type MetricsSection = "context" | "runtime" | "agents" | "jobs" | "cost" | "cache" | "extensions" | "mcp" | "files";

export interface MetricsProps {
  port: AgentPort;
  context: ContextBreakdown | null;
  mcp: McpEntry[];
  surfaces: number;
  onError: (e: unknown) => void;
  onOpenSettings?: (at?: string) => void;
  /** Which panel to land on. The rail is a column and not a router: this only
   *  decides where it scrolls, never which panels exist. */
  at?: string;
  /** The pane's own derived walk, passed down rather than re-derived here, so
   *  one derivation of "how many changed" stays the pane's. */
  changes?: Change[];
  tree?: WorkspaceChanges | null;
  yolo?: boolean;
}

/** The panels this container owns, in a fixed order. `files` reads the pane's
 *  derived walk passed in as props, not a second copy of it here, so one
 *  derivation of "how many changed" stays the pane's.
 *
 *  Fixed, and read off neither the panels' contents nor the size of anything: an
 *  order that follows the data moves a panel somewhere new the moment it has
 *  something, which is the instability a fixed order exists to remove. */
const ORDER: readonly MetricsSection[] = [
  "context", "runtime", "agents", "jobs", "cost", "cache", "extensions", "mcp", "files",
];
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import AgentsPanel from "./panels/Agents.vue";
import JobsPanel from "./panels/Jobs.vue";
import CachePanel from "./panels/Cache.vue";
import ContextPanel from "./panels/Context.vue";
import RuntimePanel from "./panels/Runtime.vue";
import ExtensionsPanel from "./panels/Extensions.vue";
import CostPanel from "./panels/Cost.vue";
import McpPanel from "./panels/Mcp.vue";
import Files from "./panels/Files.vue";

const props = defineProps<MetricsProps>();

// `at` names a panel to land on. The anchors below generate no box, so they
// exist whether or not the panel inside one drew — and the panel's own root is
// what has to be scrolled to, because a box-less element has nothing to scroll.
const box = ref<HTMLElement | null>(null);
const landing = computed(() => props.at ?? "");
watch(landing, async (id) => {
  if (!id) return;
  // A tick for the panels that would draw the target to make their first read:
  // a panel whose answer is still in flight is not in the DOM to be found.
  await nextTick();
  const anchor = box.value?.querySelector<HTMLElement>(`[data-panel="${id}"]`);
  // Optional throughout: a caller may name a panel this container does not own
  // (plan) or one that has nothing to say, and landing nowhere is the right
  // answer for both.
  anchor?.firstElementChild?.scrollIntoView({ block: "start" });
}, { immediate: true });
</script>

<template>
  <!-- The rail's own scroller, which is a real box: it is what scrolls and it is
       where the column gap between panels comes from.
       The per-panel wrappers below are display: contents so they generate no box
       of their own — a panel that draws nothing then leaves no empty gap slot,
       which a real wrapper would, and each .block stays a child of this column
       so the rail's spacing rules still reach it. -->
  <div ref="box" class="scroll">
    <div v-for="id in ORDER" :key="id" :data-panel="id" style="display: contents">
      <ContextPanel v-if="id === 'context' && context" :port="port" :context="context" :on-error="onError" />
      <RuntimePanel v-else-if="id === 'runtime'" :port="port" :on-error="onError" />
      <AgentsPanel v-else-if="id === 'agents'" :port="port" :on-error="onError" />
      <JobsPanel v-else-if="id === 'jobs'" :port="port" :on-error="onError" />
      <CostPanel v-else-if="id === 'cost'" :port="port" :on-error="onError" />
      <CachePanel v-else-if="id === 'cache'" :port="port" :on-error="onError" />
      <ExtensionsPanel v-else-if="id === 'extensions'" :port="port" :surfaces="surfaces"
        :on-error="onError" :on-open-settings="onOpenSettings" />
      <McpPanel v-else-if="id === 'mcp'" :servers="mcp" :on-open="() => onOpenSettings?.('mcp')" />
      <Files v-else-if="id === 'files'" :changes="changes ?? []" :tree="tree ?? null" :yolo="yolo ?? false" />
    </div>
  </div>
</template>