<script lang="ts">
import { t } from "../i18n";

/** How long until the flip, in the coarsest unit still true, never rounding a
 *  minute away to nothing. */
export function until(iso: string, now: number): string {
  const ms = Date.parse(iso) - now;
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const minutes = Math.max(1, Math.ceil(ms / 60000));
  if (minutes < 60) return t("common.n_min", { n: minutes });
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours >= 24) return t("composer.n_d", { n: Math.floor(hours / 24) });
  return rest ? t("composer.h_h_m_min", { h: hours, m: rest }) : t("common.n_h", { n: hours });
}
</script>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { Tariff } from "../port/session";

/** Which of the vendor's two rates this model is billed at right now. The kernel
 *  owns the schedule, so this only draws what it said, keeps the countdown
 *  honest, and asks for a fresh answer when the flip time passes. */
const props = defineProps<{ tariff: Tariff; onFlip: () => void }>();
const now = ref(Date.now());
const asked = ref("");
let timer = 0;
onMounted(() => { timer = window.setInterval(() => { now.value = Date.now(); }, 30_000); });
onUnmounted(() => window.clearInterval(timer));
const due = computed(() => props.tariff.changesAt !== undefined && Date.parse(props.tariff.changesAt) <= now.value);
watch([due, () => props.tariff.changesAt], ([d, changesAt]) => {
  if (d && changesAt && asked.value !== changesAt) {
    asked.value = changesAt;
    props.onFlip();
  }
}, { immediate: true });
const left = computed(() => (props.tariff.changesAt ? until(props.tariff.changesAt, now.value) : ""));
const label = computed(() => (props.tariff.peak ? t("composer.peak") : t("composer.off_peak")));
const title = computed(() => (props.tariff.peak
  ? left.value ? t("composer.billed_at_the_vendors_published_4", { left: left.value }) : t("composer.billed_at_the_vendors_published_3")
  : left.value ? t("composer.billed_at_the_vendors_published_2", { left: left.value }) : t("composer.billed_at_the_vendors_published")));
</script>

<template>
  <span v-if="!due" class="studio-meter-tariff" :data-peak="tariff.peak ? '' : undefined" :title="title">
    <span>{{ label }}</span>
    <i v-if="left">{{ left }}</i>
  </span>
</template>