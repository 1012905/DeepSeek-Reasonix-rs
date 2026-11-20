import { toValue, type MaybeRefOrGetter } from "vue";
import type { HubPort, TreeSession, TreeWorkspace } from "../port/hub";

/** Moving a conversation one place up or down in its project's list. The list is
 *  the kernel's, so after a move the tree is asked for again rather than patched
 *  here; while a search narrows the list there is no order to move within. */
export function useSessionMove(hub: MaybeRefOrGetter<HubPort>, reload: () => Promise<void>, onError: (e: unknown) => void, searching: MaybeRefOrGetter<boolean>) {
  const canMove = (ws: TreeWorkspace, session: TreeSession) => {
    const peers = ws.sessions.filter((s) => !!s.archived === !!session.archived);
    const at = peers.indexOf(session);
    const narrow = toValue(searching);
    return { up: !narrow && at > 0, down: !narrow && at >= 0 && at < peers.length - 1 };
  };
  const move = async (session: TreeSession, direction: -1 | 1) => {
    try {
      await toValue(hub).moveSession(session.path, direction);
      await reload();
    } catch (e) {
      onError(e);
    }
  };
  return { canMove, move };
}