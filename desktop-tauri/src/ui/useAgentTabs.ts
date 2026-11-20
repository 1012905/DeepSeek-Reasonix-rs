import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { keyOf, type Surface } from "./workbench_tabs";

export interface AgentTab { ref: string; name: string }
export interface JobTab { id: string; name: string }
export type RunTab = Extract<Surface, { kind: "agent" | "job" }>;
/** Runs to open: sub-agent runs, or one background job's output. */
export interface AgentRequest { runs?: AgentTab[]; jobs?: JobTab[]; consumed?: () => void }

type Setter<T> = (next: T | ((prev: T) => T)) => void;

/** The sub-agent runs and background jobs open as workbench tabs. A request
 *  opens each once and selects the first; the workbench reports it taken so a
 *  re-render never replays it. */
export function useAgentTabs(request: MaybeRefOrGetter<AgentRequest | undefined>, pick: (key: string) => void, setDismissed: Setter<Set<string>>) {
  const agents = ref<RunTab[]>([]);
  const setAgents: Setter<RunTab[]> = (next) => { agents.value = typeof next === "function" ? next(agents.value) : next; };
  watch(() => toValue(request), (asked0) => {
    const asked: RunTab[] = [
      ...(asked0?.runs ?? []).map((run) => ({ kind: "agent" as const, ...run })),
      ...(asked0?.jobs ?? []).map((job) => ({ kind: "job" as const, ...job })),
    ];
    if (!asked0 || asked.length === 0) return;
    const fresh = asked.filter((tab) => !agents.value.some((open) => keyOf(open) === keyOf(tab)));
    if (fresh.length) agents.value = [...agents.value, ...fresh];
    setDismissed((now) => {
      const next = new Set(now);
      for (const tab of asked) next.delete(keyOf(tab));
      return next;
    });
    pick(keyOf(asked[0]));
    asked0.consumed?.();
  }, { immediate: true });
  return { agents, setAgents };
}