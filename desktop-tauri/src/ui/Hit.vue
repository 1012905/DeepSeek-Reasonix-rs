<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ text: string; needle: string }>();

const at = computed(() => (props.needle ? props.text.toLowerCase().indexOf(props.needle.toLowerCase()) : -1));
const end = computed(() => at.value + props.needle.length);
</script>

<template>
  <template v-if="at < 0">{{ text }}</template>
  <template v-else>{{ text.slice(0, at) }}<mark class="hit">{{ text.slice(at, end) }}</mark>{{ text.slice(end) }}</template>
</template>