<script setup lang="ts">
import StudioIcon from "./StudioIcon.vue";

defineProps<{ open: boolean | undefined }>();
</script>

<template>
  <StudioIcon name="down" class="wschev ds-chev" :data-open="open ? '' : undefined" :data-leaf="open === undefined ? '' : undefined" />
  <StudioIcon name="folder" class="wsfolder ds-folder" />
</template>