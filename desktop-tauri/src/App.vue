<script setup lang="ts">
import { NConfigProvider, NTag } from "naive-ui";
import { onMounted, onUnmounted } from "vue";
import { useKernelStore } from "./stores/kernel";

const kernel = useKernelStore();
let timer: ReturnType<typeof setInterval> | undefined;

// Liveness is polled: a host exit flips the badge within one period, without
// the shell restarting anything.
onMounted(() => {
  kernel.refresh();
  timer = setInterval(() => kernel.refresh(), 2000);
});
onUnmounted(() => clearInterval(timer));
</script>

<template>
  <n-config-provider>
    <main style="padding: 24px; font-family: system-ui">
      <n-tag :type="kernel.connected ? 'success' : 'error'" :bordered="false" size="large">
        {{ kernel.statusText }}
      </n-tag>
    </main>
  </n-config-provider>
</template>
