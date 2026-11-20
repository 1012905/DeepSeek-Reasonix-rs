<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import type { AgentPort, CompactionSettings } from "../port/port";

const props = defineProps<{ port: AgentPort }>();

const tokens = (n: number) => (n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));

const box = ref<CompactionSettings | null>(null);
const used = ref<number | null>(null);

watch(() => props.port, (port) => {
  port
    .compaction()
    .then((b) => (box.value = b))
    .catch(() => (box.value = null));
  // The threshold is a number until it is next to the session's own usage,
  // and then it is a distance. A gauge that fails to load costs the reader
  // the distance, not the setting.
  port.context().then((c) => (used.value = c.used)).catch(() => (used.value = null));
}, { immediate: true });

const off = computed(() => box.value?.context_window === 0);
const capacity = computed(() => Math.round((box.value?.context_window ?? 0) * (box.value?.ratio ?? 0)));
const pct = computed(() => {
  const b = box.value;
  if (!b) return 0;
  const u = used.value;
  return b.context_window === 0 || !u ? 0 : Math.min((u / b.trigger) * 100, 100);
});
</script>

<template>
  <div v-if="!box" class="empty">{{ t("chat.the_compaction_settings_could_not") }}</div>
  <template v-else>
    <div class="cmp">
      <p v-if="off" class="note">{{ t("chat.this_source_declares_no_context") }}</p>
      <template v-else>
        <div class="cmpg">
          <div class="r">
            <span class="k">{{ t("common.now") }}</span>
            <b>{{ used === null ? "—" : tokens(used) }}</b>
          </div>
          <div class="r">
            <span class="k">{{ t("chat.next_maintenance") }}</span>
            <b>{{ tokens(box.trigger) }}</b>
          </div>
          <div class="r">
            <span class="k">{{ t("chat.model_window") }}</span>
            <b>{{ tokens(box.context_window) }}</b>
          </div>
        </div>
        <div v-if="used !== null" class="cmpbar" role="presentation">
          <i :style="{ width: `${pct}%` }" />
        </div>
        <p class="note">{{ t("chat.this_session_should_tidy_its", { n: tokens(box.trigger) }) }}</p>
      </template>
    </div>

    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("chat.capacity_guard") }}</span>
        <span class="ds">{{ t("chat.compacts_at_p_of_the", { p: String(Math.round(box.ratio * 100)) }) }}</span>
      </span>
      <span class="sc">{{ off ? "—" : tokens(capacity) }}</span>
    </div>
  </template>
</template>