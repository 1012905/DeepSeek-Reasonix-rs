<script setup lang="ts">
/** Clip is one line that may not fit. The whole string rides on `title`, so a
 *  narrow window shortens what is read rather than taking it away: an ellipsis
 *  with nothing behind it is the one form of truncation a reader cannot undo. */
defineProps<{ className?: string; children: string }>();
</script>

<template>
  <span :class="className" :title="children">{{ children }}</span>
</template>