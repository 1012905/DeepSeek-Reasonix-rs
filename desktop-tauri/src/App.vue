<script setup lang="ts">
import { NConfigProvider, NTag, NInput, NButton, NSpin } from "naive-ui";
import { onMounted, onUnmounted, ref } from "vue";
import { useKernelStore } from "./stores/kernel";
import { useStudioStore } from "./stores/studio";

const kernel = useKernelStore();
const studio = useStudioStore();
const input = ref("");
let timer: ReturnType<typeof setInterval> | undefined;

// The boot: connect the badge, open a runtime on the shell's working
// directory, and read the transcript. Liveness is polled afterwards: a host
// exit flips the badge within one period, without the shell restarting
// anything.
onMounted(() => {
  kernel.refresh();
  studio.ensureRuntime().then(() => studio.refreshHistory());
  timer = setInterval(() => kernel.refresh(), 2000);
});
onUnmounted(() => clearInterval(timer));

function send() {
  studio.submit(input.value);
  input.value = "";
}
</script>

<template>
  <n-config-provider>
    <main style="padding: 24px; font-family: system-ui; display: flex; flex-direction: column; gap: 16px">
      <n-tag :type="kernel.connected ? 'success' : 'error'" :bordered="false" size="large">
        {{ kernel.statusText }}
      </n-tag>

      <n-spin :show="studio.loading">
        <div style="display: flex; flex-direction: column; gap: 12px; min-height: 120px">
          <div v-for="(message, index) in studio.history" :key="message.msgIndex + '-' + index"
            :style="{ alignSelf: message.role === 'user' ? 'flex-end' : 'flex-start', maxWidth: '80%' }">
            <n-tag size="small" :bordered="false">{{ message.role }}</n-tag>
            <div style="white-space: pre-wrap">{{ message.content }}</div>
          </div>
          <div v-if="!studio.history.length && !studio.loading" style="opacity: 0.6">
            no conversation yet — submit a prompt below
          </div>
        </div>
      </n-spin>

      <div style="display: flex; gap: 8px">
        <n-input v-model:value="input" type="text" placeholder="describe a task…"
          @keydown.enter="send" :disabled="studio.submitting || !input.trim() || !kernel.connected" />
        <n-button type="primary" @click="send" :disabled="studio.submitting || !input.trim() || !kernel.connected">
          {{ studio.submitting ? "…" : "send" }}
        </n-button>
      </div>
      <div v-if="studio.error" style="color: #c00">{{ studio.error }}</div>
    </main>
  </n-config-provider>
</template>
