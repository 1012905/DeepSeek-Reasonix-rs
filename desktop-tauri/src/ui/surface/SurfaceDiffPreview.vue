<script setup lang="ts">
import { computed, type CSSProperties } from "vue";
import type { SurfaceDiff } from "../../port/wire";

const props = defineProps<{ diff: SurfaceDiff; label?: string }>();
const rows = computed(() => {
  const before = props.diff.before.split("\n");
  const after = props.diff.after.split("\n");
  let start = 0;
  let end = 0;
  while (start < before.length && start < after.length && before[start] === after[start]) start++;
  while (end < before.length - start && end < after.length - start && before[before.length - 1 - end] === after[after.length - 1 - end]) end++;
  return [
    ...after.slice(0, start).map(text => ({ text, sign: " " })),
    ...before.slice(start, before.length - end).map(text => ({ text, sign: "-" })),
    ...after.slice(start, after.length - end).map(text => ({ text, sign: "+" })),
    ...after.slice(after.length - end).map(text => ({ text, sign: " " })),
  ];
});
const labelStyle: CSSProperties = { fontSize: "14px", fontWeight: 500 };
const boxStyle: CSSProperties = { border: "1px solid var(--border)", padding: "12px" };
const rowStyle = (sign: string): CSSProperties => ({
  whiteSpace: "pre-wrap",
  overflowWrap: "anywhere",
  fontFamily: "var(--mono)",
  fontSize: "14px",
  color: sign === "-" ? "var(--del)" : sign === "+" ? "var(--add)" : undefined,
});
</script>

<template>
  <div class="surface-diff xv-stack">
    <p v-if="label" :style="labelStyle">{{ label }}</p>
    <div role="region" :aria-label="label" :style="boxStyle">
      <p v-for="(row, i) in rows" :key="i" class="surface-diff-row" :data-d="row.sign === ' ' ? undefined : row.sign" :style="rowStyle(row.sign)">
        <span aria-hidden="true">{{ row.sign }} </span><span>{{ row.text }}</span>
      </p>
    </div>
  </div>
</template>