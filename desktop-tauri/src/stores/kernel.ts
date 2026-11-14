import { defineStore } from "pinia";
import { ref, computed } from "vue";
// The kernel port: every call crosses to the Rust shell over Tauri IPC, and
// the shell adds the credential the loopback gate expects. The page never
// sees the token; the store only tracks liveness and exposes typed calls.
import { kernelPort } from "../port/kernel";

export const useKernelStore = defineStore("kernel", () => {
  const connected = ref(false);
  const checking = ref(false);

  const statusText = computed(() => (connected.value ? "kernel connected" : "kernel offline"));

  async function refresh() {
    checking.value = true;
    try {
      connected.value = await kernelPort.connected();
    } catch {
      connected.value = false;
    } finally {
      checking.value = false;
    }
  }

  return { connected, checking, statusText, refresh };
});
