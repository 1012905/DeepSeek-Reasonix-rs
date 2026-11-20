<script setup lang="ts">
defineProps<{ title: string; hint?: string }>();
</script>

<template>
  <section class="prefs-card">
    <header>
      <h3>{{ title }}</h3>
      <p v-if="hint">{{ hint }}</p>
    </header>
    <slot />
  </section>
</template>