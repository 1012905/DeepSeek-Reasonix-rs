import { onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, WorkspaceChange } from "../port/port";

// How long writes are let to pile up before a repository is read: one agent turn
// is dozens of tool calls, and the repository is read once for all of them.
export const SETTLE_MS = 700;

// The changes of one repository below the workspace. A repository that is
// collapsed is not read at all, and is read the moment it is opened; the first
// read and a read the person asked for (reload) are immediate; reads that the
// agent's writes ask for wait until the writes settle. A read asked for while one
// is running is made once more when that one ends, never alongside it.
export function useRepoChanges(port: MaybeRefOrGetter<AgentPort>, repo: MaybeRefOrGetter<string>, open: MaybeRefOrGetter<boolean>, wrote: MaybeRefOrGetter<number>) {
  const changes = ref<WorkspaceChange[]>([]);
  const asked = ref(0);
  let flying = false;
  let again = false;
  let live = true;
  let immediate = true;
  onScopeDispose(() => { live = false; });

  const read = () => {
    if (flying) {
      again = true;
      return;
    }
    flying = true;
    toValue(port).changes(toValue(repo)).then(
      (r) => { if (live) changes.value = r.changes; },
      () => { if (live) changes.value = []; },
    ).finally(() => {
      flying = false;
      if (again && live) {
        again = false;
        read();
      }
    });
  };

  watch(() => toValue(open), (on) => { if (on) immediate = true; }, { immediate: true });
  watch([() => toValue(open), () => toValue(wrote), asked, () => toValue(port), () => toValue(repo)], (_v, _was, onCleanup) => {
    if (!toValue(open)) return;
    const wait = immediate ? 0 : SETTLE_MS;
    immediate = false;
    const timer = setTimeout(read, wait);
    onCleanup(() => clearTimeout(timer));
  }, { immediate: true });

  const reload = () => {
    immediate = true;
    asked.value = asked.value + 1;
  };
  return { changes, reload };
}