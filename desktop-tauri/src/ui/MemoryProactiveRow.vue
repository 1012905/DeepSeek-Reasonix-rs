<script setup lang="ts">
import { onWatcherCleanup, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import type { MemoryProactive } from "../port/memory";

const MODES: [MemoryProactive, string][] = [["off", "memory.proactive_off"], ["ask", "memory.proactive_ask"], ["auto", "memory.proactive_auto"]];

// One line at the top of the memory page: what the model does when it sees
// something worth keeping for good.
const props = defineProps<{ port: AgentPort }>();

const mode = ref<MemoryProactive | null>(null);
const error = ref("");
watch(() => props.port, (port) => {
  let live = true;
  port.memoryProactive().then((m) => { if (live) mode.value = m; }).catch((e) => { if (live) error.value = reason(e); });
  onWatcherCleanup(() => { live = false; });
}, { immediate: true });
const pick = (next: MemoryProactive) => {
  const was = mode.value;
  mode.value = next;
  error.value = "";
  props.port.setMemoryProactive(next).catch((e) => { mode.value = was; error.value = reason(e); });
};
</script>

<template>
  <div v-if="mode !== null || error" class="memproactive">
    <div>
      <b>{{ t("memory.proactive_title") }}</b>
      <span>{{ t("memory.proactive_hint") }}</span>
      <span v-if="error" role="alert" class="memproactive-err">{{ error }}</span>
    </div>
    <div class="memproactive-seg" role="radiogroup" :aria-label="t('memory.proactive_title')">
      <button v-for="[id, label] in MODES" :key="id" type="button" role="radio" :aria-checked="mode === id" :data-on="mode === id ? '' : undefined" data-action="memory.proactive" :data-target="id" @click="pick(id)">{{ t(label) }}</button>
    </div>
  </div>
</template>