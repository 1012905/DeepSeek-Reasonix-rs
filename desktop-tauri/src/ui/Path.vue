<script setup lang="ts">
import { computed } from "vue";

/** A path that breaks after its separators.
 *
 *  `word-break: break-all` cuts through the middle of a token, which on a long
 *  workspace path lands inside a number. <wbr> is markup rather than text, so
 *  copying the row still yields the path and nothing else. */
const props = defineProps<{ of: string }>();

const parts = computed(() => props.of.split("/"));
</script>

<template>
  <template v-for="(seg, i) in parts" :key="i">{{ i ? "/" : "" }}{{ seg }}<wbr v-if="i < parts.length - 1" /></template>
</template>