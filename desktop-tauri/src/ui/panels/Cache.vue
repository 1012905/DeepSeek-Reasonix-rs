<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the Cache panel needs. It reads the session's own ledger rather than a
 *  derived summary: hit and miss move once per request, and a second reading of
 *  them is a second thing that can be wrong. */
export interface CachePanelProps {
  port: AgentPort;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import { pct, tokens } from "../../i18n/format";
import type { SessionUsage } from "../../port/session";
import { cacheRate, warmCacheRate } from "../../state/overview";

// The hit ratio is session-cumulative, which is the one shape that cannot do the
// job its size used to claim: the longer a session runs, the less a broken turn
// can move it. So it is a figure beside the name and not the block's headline.
//
// Like every other panel here, it reads the pane's status rather than opening a
// stream of its own: a second subscription per panel is a second connection
// against an origin that allows six, and streamBudget hands out the ones there
// are. What that costs is the per-request diagnostics that only ride usage
// frames — a prefix having moved is the transcript's note, not this panel's.
const props = defineProps<CachePanelProps>();

const hit = ref(0);
const miss = ref(0);
const unknown = ref(0);
const warm = ref<SessionUsage | null>(null);
const failed = ref("");

const read = () => {
  props.port
    .status()
    .then((st) => {
      hit.value = st.cacheHit ?? 0;
      miss.value = st.cacheMiss ?? 0;
      unknown.value = st.sessionUsage?.cacheUnknown ?? 0;
      warm.value = st.sessionUsage ?? null;
      failed.value = "";
    })
    .catch((e: unknown) => {
      // An unread ledger leaves the last figures standing rather than blanking
      // them: what was read is still true, and a row of dashes says nothing.
      failed.value = reason(e);
      props.onError(e);
    });
};

watch(() => props.port, read, { immediate: true });

const up = computed(() => hit.value + miss.value);
// Nothing has been asked yet, so there is nothing to report about it: a bar with
// no denominator behind it is a shape, not a reading.
const speaks = computed(() => up.value > 0);
const rate = computed(() => cacheRate(hit.value, miss.value, unknown.value));
// Warm is the rate with the cold start taken out of hit, miss and unknown alike:
// null until a request after it split its prompt.
const warmRate = computed(() => (warm.value ? warmCacheRate(warm.value) : null));
</script>

<template>
  <div v-if="speaks || failed" class="block" data-b="cache">
    <div class="lbl">
      <span>{{ t("panels.prefix_cache") }}</span>
      <span v-if="rate !== null" class="c">{{ pct(rate, 1) }}</span>
    </div>
    <div v-if="rate !== null" class="bar">
      <i class="c" :style="{ flexGrow: Math.max(rate * 100, 0.4) }" />
      <i class="f" :style="{ flexGrow: Math.max(100 - rate * 100, 0.4) }" />
    </div>
    <div class="nums">
      <span>{{ t("panels.hit") }}<b>{{ tokens(hit) }}</b></span>
      <span>{{ t("panels.missed") }}<b>{{ tokens(miss) }}</b></span>
    </div>
    <!-- The cold start is the session's opening request, which pays for the
         prefix every later one reuses; crediting it forever flatters the rate on
         a session that has only just begun. -->
    <div v-if="warmRate !== null" class="drow">
      <span class="k">{{ t("panels.reused") }}</span>
      <span class="v">{{ pct(warmRate, 1) }}</span>
    </div>
    <p v-if="failed" class="ctxnote" role="alert">{{ failed }}</p>
  </div>
</template>
