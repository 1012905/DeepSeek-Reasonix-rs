import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, WorkspaceBranch, WorkspaceGit } from "../port/port";

export function useWorkspaceBranches(port: MaybeRefOrGetter<AgentPort>, onChanged: () => void, onSwitched: ((git: WorkspaceGit) => void) | undefined, onError: (error: unknown) => void, root: MaybeRefOrGetter<string> = "") {
  const locals = ref<WorkspaceBranch[]>([]);
  const moving = ref<string | null>(null);
  const reading = ref<"idle" | "loading" | "ready" | "failed">("idle");
  let generation = 0;
  let asked = false;
  const loadBranches = () => {
    const current = generation;
    asked = true;
    reading.value = "loading";
    void toValue(port).branches().then((result) => { if (current === generation) { asked = false; locals.value = result.branches; reading.value = "ready"; } }).catch((error) => { if (current === generation) { asked = false; reading.value = "failed"; onError(error); } });
  };
  watch([() => toValue(port), () => toValue(root)], () => {
    generation++; locals.value = []; moving.value = null; reading.value = "idle";
    if (asked) loadBranches();
    return () => { generation++; };
  }, { immediate: true });
  const switchTo = (name: string) => {
    if (moving.value !== null) return;
    const current = generation;
    moving.value = name;
    void toValue(port).switchBranch(name).then((git) => {
      if (current !== generation) return;
      onChanged(); onSwitched?.(git);
    }).catch((error) => { if (current === generation) onError(error); })
      .finally(() => { if (current === generation) moving.value = null; });
  };
  return { locals, moving, reading, loadBranches, switchTo };
}