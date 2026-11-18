import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, WorkspaceInfo } from "../port/port";

export function useWorkspaceInfo(port: MaybeRefOrGetter<AgentPort | null>, root: MaybeRefOrGetter<string>, ready: MaybeRefOrGetter<boolean>) {
  const read = ref<{ port: AgentPort; root: string; value: WorkspaceInfo } | null>(null);
  watch([() => toValue(port), () => toValue(root), () => toValue(ready)], ([agent, at, go]) => {
    if (!agent || !at || !go) return;
    let current = true;
    read.value = null;
    void agent.workspaces().then((value) => {
      if (current) read.value = { port: agent, root: at, value };
    }).catch(() => {});
    return () => { current = false; };
  }, { immediate: true });
  return computed(() => {
    const held = read.value;
    return toValue(ready) && held?.port === toValue(port) && held.root === toValue(root) ? held.value : null;
  });
}