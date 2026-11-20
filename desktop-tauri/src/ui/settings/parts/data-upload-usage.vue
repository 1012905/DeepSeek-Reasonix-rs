<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { bytes, count, pct } from "../../../i18n/format";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { UploadReport } from "../../../port/usage";

// What model providers were sent in the last month. Every request carries the
// whole conversation, so this grows with context size times request count and not
// with how much was typed. The numbers are request bodies as they went out,
// before the connection's own overhead; downloads are not counted.
const DAYS = 30;

const props = defineProps<{ port: AgentPort }>();

const report = ref<UploadReport | null>(null);
const failed = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.uploadUsage(DAYS)
    .then((r) => { if (live) report.value = r; }, (e) => { if (live) failed.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

const today = computed(() => report.value?.daily[report.value.daily.length - 1]?.bytes ?? 0);
const average = computed(() => (report.value && report.value.requests > 0 ? Math.round(report.value.bytes / report.value.requests) : 0));
const totalSaid = computed(() => (report.value
  ? t("usage.size_in_n_requests_avg", { size: bytes(report.value.bytes), n: count(report.value.requests), avg: bytes(average.value) })
  : ""));
</script>

<template>
  <p class="note">{{ t("usage.the_size_of_the_request") }}</p>
  <p v-if="failed" class="note" role="alert">{{ failed }}</p>
  <p v-else-if="!report" class="note">{{ t("common.loading") }}</p>
  <p v-else-if="report.requests === 0" class="note">{{ t("usage.nothing_recorded_yet_counting_starts") }}</p>
  <template v-else>
    <div class="lrow">
      <span class="tx"><span class="lb">{{ t("usage.uploaded_today") }}</span></span>
      <span class="sc">{{ bytes(today) }}</span>
    </div>
    <div class="lrow">
      <span class="tx"><span class="lb">{{ t("usage.uploaded_in_the_last_n", { n: DAYS }) }}</span></span>
      <span class="sc">{{ totalSaid }}</span>
    </div>
    <div v-for="p in report.providers" :key="p.provider" class="lrow">
      <span class="tx"><span class="lb">{{ p.provider }}</span></span>
      <span class="sc">{{ `${bytes(p.bytes)} · ${pct(p.percent / 100)} · ${count(p.requests)}` }}</span>
    </div>
  </template>
</template>
