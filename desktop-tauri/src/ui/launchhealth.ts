import { onWatcherCleanup, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, ProviderSetup } from "../port/port";

// How long a launch has to stay up before it may retire the update it booted
// from. The desktop this replaced waited the same two seconds for the same
// reason: a build that comes up and dies immediately must not be the one that
// throws away the way back to the build before it.
export const PROBATION_MS = 2000;

/** Says this launch works, once, after it has stayed up long enough to mean it.
 *
 *  That sentence is what retires the update the launch booted from. The swap is
 *  performed by a process that cannot judge the result, so the rollback
 *  material is kept until the application that came up reports in — and nothing
 *  in this tree reported. The endpoint existed with no caller, so the
 *  transaction was never closed and every install after the first refused: a
 *  pending update already exists.
 *
 *  Said from the page rather than the shell because a window that opened and
 *  then failed to render is exactly the case the rollback exists for, and only
 *  the page knows it got past that. Silent either way: nothing here was asked
 *  for by anyone, and a launch that booted from no update has nothing to retire.
 */
export function useLaunchHealth(port: MaybeRefOrGetter<AgentPort | null>, setup: MaybeRefOrGetter<ProviderSetup | null | undefined>, welcomed: MaybeRefOrGetter<boolean | undefined>) {
  let said = false;
  watch([() => toValue(port), () => toValue(setup), () => toValue(welcomed)], ([p, s, w]) => {
    if (said || !p) return;
    // Onboarding is not "up": an application still asking for a key is one the
    // user cannot use yet, and retiring the way back from inside it discards
    // the rollback on a launch that has proven nothing.
    if (s === undefined || w === undefined || !w || s?.required) return;
    const timer = setTimeout(() => {
      said = true;
      void p.acknowledgeLaunchHealth();
    }, PROBATION_MS);
    onWatcherCleanup(() => clearTimeout(timer));
  }, { immediate: true });
}

/** Reads what the active machine says about setup and the welcome. A machine
 *  that cannot answer has met the app before as far as we care: the sequence
 *  must never be what stands between someone and their session. */
export function useFirstRunRead(port: MaybeRefOrGetter<AgentPort | null>, setSetup: (v: ProviderSetup | null) => void, setWelcomed: (v: boolean) => void) {
  watch(() => toValue(port), (p) => {
    if (!p) return;
    let alive = true;
    p.providerSetup().then((v) => alive && setSetup(v)).catch(() => alive && setSetup(null));
    p.welcomeSeen().then((v) => alive && setWelcomed(v)).catch(() => alive && setWelcomed(true));
    onWatcherCleanup(() => {
      alive = false;
    });
  }, { immediate: true });
}