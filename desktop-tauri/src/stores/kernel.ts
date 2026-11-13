import { defineStore } from "pinia";
import { ref, computed } from "vue";

export const useKernelStore = defineStore("kernel", () => {
  const connected = ref(false);

  const statusText = computed(() => (connected.value ? "kernel connected" : "kernel offline"));

  function setConnected(value: boolean) {
    connected.value = value;
  }

  return { connected, statusText, setConnected };
});
