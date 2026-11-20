import { onScopeDispose, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, Checkpoint } from "../port/port";

const RETRY_MS = 1000;

// The kernel opens a turn's checkpoint when the turn starts and fills it while
// it runs, so both turn edges change the list, and so does a move to another
// known session. The first status naming the session the mount read already
// covered changes nothing. No later edge is owed to a failed read, so it is
// tried once more; a read a newer one has superseded is dropped.
export function useCheckpointRefresh(port: MaybeRefOrGetter<AgentPort>, sessionPath: MaybeRefOrGetter<string | undefined>, running: MaybeRefOrGetter<boolean>, set: (cps: Checkpoint[]) => void) {
  let seen: { port: AgentPort; path: string | undefined; running: boolean } | null = null;
  let latest = 0;
  let alive = true;
  onScopeDispose(() => { alive = false; });
  watch([() => toValue(port), () => toValue(sessionPath), () => toValue(running)], () => {
    const agent = toValue(port), path = toValue(sessionPath), run = toValue(running);
    const was = seen;
    seen = { port: agent, path, running: run };
    if (was && was.port === agent && (was.path === undefined || was.path === path) && was.running === run) return;
    const mine = ++latest;
    const current = () => alive && latest === mine;
    const read = (retry: boolean) => {
      agent.checkpoints().then(
        (cps) => current() && set(cps),
        () => retry && current() && setTimeout(() => current() && read(false), RETRY_MS),
      );
    };
    read(true);
  }, { immediate: true });
}