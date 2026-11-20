import { shallowRef, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort } from "../port/port";
import type { AgentRequest } from "./useAgentTabs";

/** Opening a task card's sub-agents in the workbench. The kernel's list pairs a
 *  run with the tool call that spawned it, so the card names its runs exactly.
 *  False means this call has no run to read — an older session, or a kernel
 *  without the route — and the caller keeps its own view. */
export function useAgentRequest(port: MaybeRefOrGetter<AgentPort>, reveal: () => void) {
  // consumed clears by identity; a deep ref would hand back a proxy, never the request it was given.
  const request = shallowRef<AgentRequest | undefined>(undefined);
  watch(() => toValue(port), () => { request.value = undefined; });
  const open = async (call: string): Promise<boolean> => {
    let runs;
    try {
      runs = (await toValue(port).subagents()).filter((run) => run.parentCall === call);
    } catch {
      return false;
    }
    if (runs.length === 0) return false;
    const req: AgentRequest = {
      runs: runs.map((run) => ({ ref: run.ref, name: run.name })),
      consumed: () => { if (request.value === req) request.value = undefined; },
    };
    request.value = req;
    reveal();
    return true;
  };
  const openJob = (job: { id: string; name: string }) => {
    const req: AgentRequest = { jobs: [job], consumed: () => { if (request.value === req) request.value = undefined; } };
    request.value = req;
    reveal();
  };
  return { request, open, openJob };
}