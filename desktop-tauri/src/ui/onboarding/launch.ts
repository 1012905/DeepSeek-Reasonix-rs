import type { AgentPort } from "../../port/port";

/** What an earlier launch left behind, as the guide reports it.
 *
 *  `names` are the record files themselves rather than a count: the point of
 *  showing them at all is that the person can go and read one. `more` says the
 *  kernel held some back, so the list is not mistaken for the whole of it. */
export interface LaunchCrashes {
  dir: string;
  names: string[];
  more: boolean;
}

/** Reads the abnormal-exit records earlier launches left behind.
 *
 *  This is a read the shared launch-health composable does not make: that one
 *  closes out the update transaction once a launch is up, while these files are
 *  what a kernel that died mid-turn wrote. A machine with none, or one whose
 *  kernel cannot answer the question, is the ordinary case and answers null — a
 *  guide that opened on an error where a list of records should be would be
 *  worse than one that says nothing.
 */
export function readCrashes(port: Pick<AgentPort, "studioLogs">): Promise<LaunchCrashes | null> {
  return port
    .studioLogs()
    .then((logs) => {
      const names = (logs.crashes ?? []).map((c) => c.name).filter(Boolean);
      if (names.length === 0) return null;
      return { dir: logs.crashDir || logs.dir, names, more: !!logs.truncated };
    })
    .catch(() => null);
}

/** Says this launch works, which is what retires the update it booted from.
 *
 *  Failure is silent on purpose: nothing the person asked for is happening, and
 *  the two ordinary answers are both non-events — a kernel with no update
 *  capability has no route, and a launch that booted from no update has nothing
 *  to retire. `update.go` answers 204 either way.
 */
export function acknowledgeLaunch(port: Pick<AgentPort, "acknowledgeLaunchHealth">): Promise<void> {
  return port.acknowledgeLaunchHealth().catch(() => {});
}
