import { defineStore } from "pinia";
import { ref } from "vue";
// The Studio content store: runtime boot, history and turn submission — all
// through the kernel port (Tauri IPC; the Rust shell adds the credential).
// A submitted turn returns 202; its output lands in the history, so
// submitting ends with a history refresh.
import { kernelPort } from "../port/kernel";

export interface HistoryMessage {
  role: string;
  content: string;
  reasoning?: string;
  msgIndex: number;
}

export interface RuntimeView {
  id: string;
  base: string;
}

export const useStudioStore = defineStore("studio", () => {
  const history = ref<HistoryMessage[]>([]);
  const runtimeId = ref<string>("");
  const loading = ref(false);
  const submitting = ref(false);
  const error = ref("");

  async function refreshHistory() {
    loading.value = true;
    try {
      history.value = await kernelPort.get<HistoryMessage[]>("/history");
      error.value = "";
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    } finally {
      loading.value = false;
    }
  }

  // The host routes unprefixed paths to the first open runtime, so the boot
  // only has to make sure one exists: open it on the shell's own working
  // directory (the project the user launched the shell in).
  async function ensureRuntime() {
    const runtimes = await kernelPort.get<RuntimeView[]>("/runtimes");
    if (runtimes.length > 0) {
      runtimeId.value = runtimes[0].id;
      return;
    }
    const root = await kernelPort.defaultRoot();
    const opened = await kernelPort.post<RuntimeView>("/runtimes", { root });
    runtimeId.value = opened.id;
  }

  async function submit(input: string) {
    if (!input.trim()) return;
    submitting.value = true;
    try {
      await kernelPort.post("/submit", { input });
      await refreshHistory();
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    } finally {
      submitting.value = false;
    }
  }

  return {
    history,
    runtimeId,
    loading,
    submitting,
    error,
    refreshHistory,
    ensureRuntime,
    submit,
  };
});
