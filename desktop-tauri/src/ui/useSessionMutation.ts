import { ref, watch } from "vue";

export type SessionMutation = "pin" | "archive" | "delete" | "read";

export function captureSessionFocus(row: HTMLElement | null): () => void {
  const from = document.activeElement as HTMLElement | null;
  if (!row || !from || !row.contains(from)) return () => {};
  const rows = [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')];
  const at = rows.indexOf(row);
  const neighbors = [rows[at + 1], rows[at - 1]];
  const target = row.dataset.target;
  return () => {
    if (document.activeElement !== from && document.activeElement !== document.body) return;
    const replacement = target ? [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find((node) => node.dataset.target === target) : null;
    const candidates = [from, row, replacement, ...neighbors,
      document.querySelector<HTMLElement>('[data-action="workspace.menu"]')];
    candidates.find((node) => node?.isConnected && !node.matches(":disabled"))?.focus({ preventScroll: true });
  };
}

export function useSessionMutation(onError: (error: unknown) => void) {
  const pending = ref<ReadonlyMap<string, SessionMutation>>(new Map());
  const flights = new Set<string>();
  const finished: (() => void)[] = [];
  watch(pending, () => { for (const restore of finished.splice(0)) restore(); }, { flush: "post" });
  const run = async (key: string, kind: SessionMutation, row: HTMLElement | null, task: () => void | Promise<void>, restore = captureSessionFocus(row)) => {
    if (flights.has(key)) return;
    flights.add(key);
    pending.value = new Map(pending.value).set(key, kind);
    try { await task(); }
    catch (error) { onError(error); }
    finally {
      flights.delete(key);
      finished.push(restore);
      const next = new Map(pending.value);
      next.delete(key);
      pending.value = next;
    }
  };
  return { pending, run };
}