<script lang="ts">
import { computed, onUnmounted, ref, toValue, watch, type ComputedRef, type MaybeRefOrGetter } from "vue";

// 墙钟跨三个数量级 —— 一次读文件 0.2s，一条 bash 一分钟 —— 线性映射会把快的
// 那一批全挤成一个点。对数摊开它们，条只说长短，是什么类别由轨道那条线说。
function tookWidth(ms: number) {
  return 3 + Math.log10(1 + (ms / 1000) * 9) * 22;
}

// A wait that does not say how long it has been waiting reads the same at two
// seconds and at two minutes. The kernel's startedAt is deliberately not used —
// it is unix ms from the kernel's clock, and over a remote session that is not
// this window's. This counts on the window's own clock from the moment the
// window saw the call running.
export function useWatched(running: MaybeRefOrGetter<boolean>): ComputedRef<number> {
  const from = ref(0);
  const now = ref(0);
  let id: ReturnType<typeof setInterval> | undefined;
  watch(() => toValue(running), (r) => {
    if (id !== undefined) { clearInterval(id); id = undefined; }
    if (!r) {
      from.value = 0;
      now.value = 0;
      return;
    }
    from.value = Date.now();
    now.value = Date.now();
    // One second is the resolution a reader acts on; anything finer is a digit
    // changing for its own sake.
    id = setInterval(() => (now.value = Date.now()), 1000);
  }, { immediate: true });
  onUnmounted(() => { if (id !== undefined) clearInterval(id); });
  return computed(() => (from.value && now.value ? now.value - from.value : 0));
}
</script>

<script setup lang="ts">
import type { Tool } from "../port/wire";
import { t } from "../i18n";
import { seconds, tokens } from "../i18n/format";

// What a step cost, in the two units the kernel can state honestly per call:
// wall-clock, and what the call left in the prompt. The spec pairs them in that
// order wherever both are known ("1m04s · 9.5k"), so every card reads the same way.
const props = withDefaults(defineProps<{ tools: Tool[]; running?: boolean }>(), { running: false });

const ms = computed(() => props.tools.reduce((n, tool) => n + (tool.durationMs ?? 0), 0));
const left = computed(() => props.tools.reduce((n, tool) => n + (tool.contextTokens ?? 0), 0));
const watched = useWatched(() => props.running);
// Under a second there is nothing to report: a call that answers that fast
// never looked stuck, and a digit appearing and going is its own noise.
const live = computed(() => (watched.value >= 1000 ? watched.value : 0));
</script>

<template>
  <span v-if="ms || left || live" class="cost">
    <i v-if="ms > 0" class="took" :style="{ width: `${tookWidth(ms).toFixed(1)}px` }" aria-hidden="true" />
    <span v-if="ms > 0">{{ seconds(ms) }}</span>
    <!-- No bar while it runs. The bar is a length against a known duration;
         growing one against a duration nobody knows yet is a progress bar that
         cannot be honest about what fraction it has drawn. -->
    <span v-if="!ms && live > 0" class="live" :title="t('composer.how_long_this_step_has')">{{ seconds(live, 0) }}</span>
    <span v-if="left > 0" :title="t('composer.estimated_tokens_this_step_leaves')">{{ tokens(left) }}</span>
  </span>
</template>