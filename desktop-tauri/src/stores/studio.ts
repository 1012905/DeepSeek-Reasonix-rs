import { defineStore } from "pinia";
import { ref } from "vue";
// The Studio content store: history and turn submission, both through the
// kernel port (Tauri IPC — the Rust shell adds the credential). A submitted
// turn returns 202; its output lands in the history, so submitting ends with
// a history refresh.
import { kernelPort } from "../port/kernel";

export interface HistoryMessage {
  role: string;
  content: string;
  reasoning?: string;
  msgIndex: number;
}

export const useStudioStore = defineStore("studio", () => {
  const history = ref<HistoryMessage[]>([]);
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

  return { history, loading, submitting, error, refreshHistory, submit };
});
