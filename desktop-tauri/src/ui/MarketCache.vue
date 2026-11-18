<script setup lang="ts">
import { computed } from "vue";
import { current, t } from "../i18n";
import type { MarketCache } from "../port/port";

const props = defineProps<{ cache: MarketCache; detail?: boolean; onRetry: () => void }>();

function savedAt(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(current() === "zh" ? "zh-CN" : "en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

const when = computed(() => savedAt(props.cache.cachedAt));
</script>

<template>
  <div class="find" data-cached="" role="status">
    <span class="t">{{ t("ext.showing_cached_data") }}</span>
    <span class="why">
      {{ cache.cause === "unreachable"
        ? t("ext.the_community_market_could_not", { when })
        : t("ext.the_community_market_is_not", { when }) }}
    </span>
    <div class="acts">
      <button class="act" :data-action="detail ? 'market.detail-retry' : 'market.retry'" @click="onRetry">{{ t("common.retry") }}</button>
    </div>
  </div>
</template>