<script setup lang="ts">
import { tokenizeJson } from "./jsonTokens";

/** JSON text with keys, strings, numbers and literals coloured by the same
 *  syntax tokens a fenced code block uses. The text is unchanged: selecting and
 *  copying it yields exactly what the model sent. */
defineProps<{ text: string }>();
</script>

<template>
  <template v-for="(tok, i) in tokenizeJson(text)" :key="i">
    <span v-if="tok.kind !== 'plain'" :class="`jt-${tok.kind}`">{{ tok.text }}</span>
    <template v-else>{{ tok.text }}</template>
  </template>
</template>