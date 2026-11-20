import { computed, toValue, watch, type MaybeRefOrGetter } from "vue";

interface Args {
  settings: MaybeRefOrGetter<boolean>;
  feedback: MaybeRefOrGetter<boolean>;
  panesRead: MaybeRefOrGetter<boolean>;
  panes: MaybeRefOrGetter<number>;
  root: MaybeRefOrGetter<string | undefined>;
  openPane: (req: { root?: string }) => Promise<unknown>;
  fail: (e: unknown) => void;
}

// Settings and feedback are answered by a pane's kernel, so with none open
// there is nothing to ask. Opening either then starts a session in the current
// folder, or in the app's own workspace when there is no folder.
export function usePaneForSheets(a: Args) {
  let started = false;
  const wants = computed(() => toValue(a.settings) || toValue(a.feedback));
  watch([wants, () => toValue(a.panesRead), () => toValue(a.panes), () => toValue(a.root)], () => {
    const panes = toValue(a.panes), root = toValue(a.root), panesRead = toValue(a.panesRead);
    if (!wants.value || !panesRead || panes > 0) {
      started = false;
      return;
    }
    if (started) return;
    started = true;
    void a.openPane({ root }).catch(a.fail);
  }, { immediate: true });
}