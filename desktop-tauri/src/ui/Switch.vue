<script setup lang="ts">
// The one on/off control in the settings pane: a durable switch, not a
// command. Everything that can be turned back on by clicking again is one.
withDefaults(defineProps<{
  on: boolean; busy?: boolean; label: string; onClick: () => void;
}>(), { busy: false });
</script>

<template>
  <button type="button" class="sw" role="switch" :aria-checked="on" :aria-label="label" :disabled="busy" @click="onClick">
    <i />
  </button>
</template>