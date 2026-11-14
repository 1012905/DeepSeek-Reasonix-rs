import { defineStore } from "pinia";
import { ref } from "vue";
// The Studio content store: runtime boot, history, turn submission and the
// live event stream — all through the kernel port (Tauri IPC; the Rust shell
// adds the credential). A submitted turn returns 202; its output arrives on
// the kernel-frame stream the shell bridges, and lands durably in the
// history, so turn_done ends with a history refresh.
import { listen } from "@tauri-apps/api/event";
import { kernelPort } from "../port/kernel";
import { KINDS } from "../port/wire";

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

// One live frame, narrowed to the fields the transcript renders. Frames
// carry the wire event JSON verbatim; unknown kinds render as log lines so
// nothing the kernel sends disappears silently.
export interface LiveEvent {
  kind: string;
  text?: string;
  reasoning?: string;
  detail?: string;
  err?: string;
}

export const useStudioStore = defineStore("studio", () => {
  const history = ref<HistoryMessage[]>([]);
  const runtimeId = ref<string>("");
  const live = ref<LiveEvent[]>([]);
  const loading = ref(false);
  const submitting = ref(false);
  const error = ref("");
  let unlisten: (() => void) | undefined;

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

  // The Rust shell bridges the kernel's /events stream and re-emits each
  // frame as kernel-frame; this subscribes once and narrows the frames to
  // the fields the transcript renders. turn_done refreshes the durable
  // transcript, which by then carries what the stream just showed.
  async function startStream() {
    if (unlisten) return;
    unlisten = await listen<string>("kernel-frame", (e) => {
      let frame: LiveEvent;
      try {
        frame = JSON.parse(e.payload) as LiveEvent;
      } catch {
        return;
      }
      if (!(KINDS as readonly string[]).includes(frame.kind)) return;
      live.value.push({
        kind: frame.kind,
        text: frame.text,
        reasoning: frame.reasoning,
        detail: frame.detail,
        err: frame.err,
      });
      if (frame.kind === "turn_done") void refreshHistory();
    });
  }

  function stopStream() {
    unlisten?.();
    unlisten = undefined;
  }

  return {
    history,
    runtimeId,
    live,
    loading,
    submitting,
    error,
    refreshHistory,
    ensureRuntime,
    submit,
    startStream,
    stopStream,
  };
});
