import { computed, reactive, ref } from "vue";

export interface SessionSwitch {
  // The row being opened; empty when nothing is, or when what opens has no row yet.
  path: string;
  // True until the opened pane has taken focus; the selection outlives it.
  pending: boolean;
  opening: (path: string) => boolean;
  begin: (path: string) => number;
  current: (ticket: number) => boolean;
  landed: (ticket: number) => void;
  end: (ticket: number) => void;
  cancel: () => void;
}

interface Held {
  path: string;
  pending: boolean;
}

// Owns "which session is being opened". Selection follows the click and a
// ticket orders concurrent opens, so only the last one decides focus and a
// refusal from a superseded open leaves the newer selection alone.
export function useSessionSwitch(): SessionSwitch {
  const held = ref<Held | null>(null);
  let latest = 0;
  let path: string | null = null;

  const begin = (next: string) => {
    latest += 1;
    path = next;
    held.value = { path: next, pending: true };
    return latest;
  };
  const opening = (candidate: string) => candidate !== "" && path === candidate;
  const current = (ticket: number) => ticket === latest;
  const landed = (ticket: number) => {
    if (ticket === latest) held.value = held.value ? { ...held.value, pending: false } : held.value;
  };
  const end = (ticket: number) => {
    if (ticket !== latest) return;
    path = null;
    held.value = null;
  };
  const cancel = () => {
    latest += 1;
    path = null;
    held.value = null;
  };

  return reactive({
    path: computed(() => held.value?.path ?? ""),
    pending: computed(() => held.value?.pending ?? false),
    opening,
    begin,
    current,
    landed,
    end,
    cancel,
  });
}