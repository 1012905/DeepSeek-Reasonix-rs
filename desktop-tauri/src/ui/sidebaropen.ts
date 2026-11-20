import { toValue, type MaybeRefOrGetter } from "vue";

export interface SidebarOpenRequest {
  root?: string;
  sessionPath?: string;
}

export interface SidebarRuntime {
  id: string;
  sessionPath?: string;
}

type SetStateAction<T> = T | ((prev: T) => T);
type Dispatch<A> = (value: A) => void;

/** Picking a conversation that is already on screen brings it back into view
 *  rather than re-opening it: the pane may be sitting on its analysis or
 *  workbench page, and the click is the way out of that page.
 *
 *  It lives here rather than in `App` because it is a rule about panes, not a
 *  wiring detail of the window — and because `App` is already at its size
 *  ceiling. */
export function useSidebarOpen<V extends string>(
  runtimes: MaybeRefOrGetter<SidebarRuntime[]>,
  openPane: (req: SidebarOpenRequest) => Promise<unknown>,
  setActive: (id: string) => void,
  setViewRequests: Dispatch<SetStateAction<Record<string, { to: V; n: number }>>>,
  reading: MaybeRefOrGetter<V>,
) {
  return (req: SidebarOpenRequest): Promise<void> => {
    const held = req.sessionPath ? toValue(runtimes).find((rt) => rt.sessionPath === req.sessionPath) : undefined;
    if (!held) {
      // The caller awaits this to know an open failed; `void` here would
      // swallow the rejection and leave the row stuck on its old selection.
      return openPane(req).then(() => undefined);
    }
    setActive(held.id);
    setViewRequests((requests) => ({
      ...requests,
      [held.id]: { to: toValue(reading), n: (requests[held.id]?.n ?? 0) + 1 },
    }));
    return Promise.resolve();
  };
}