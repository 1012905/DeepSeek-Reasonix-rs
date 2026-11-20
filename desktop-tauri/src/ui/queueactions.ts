import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, Queue as QueueSnapshot, QueueItem } from "../port/port";
import type { SessionEvent } from "../state/session";
import { localId } from "../state/session";

interface Inputs {
  port: MaybeRefOrGetter<AgentPort>;
  dispatch: (event: SessionEvent) => void;
  fail: (e: unknown) => void;
  moved: MaybeRefOrGetter<number>;
  sessionState?: MaybeRefOrGetter<"pending" | "settled">;
  sessionPath?: MaybeRefOrGetter<string>;
}

/** The outbox as the kernel holds it, and what can be done to a line still in
 *  it: send it now, make it guidance, take it back, retry it. Nothing here patches the snapshot: every edit is asked
 *  of the kernel and the answer is read back whole, which is also what puts
 *  another window's lines in front of this one. */
export function useQueueActions({ port, dispatch, fail, moved, sessionState, sessionPath }: Inputs) {
  const queue = ref<QueueSnapshot | null>(null);
  // The last line taken back, numbered so the same text twice is two requests.
  const restored = ref({ n: 0, text: "" });
  const onRestoreText = (text: string) => { restored.value = { n: restored.value.n + 1, text }; };
  // The queue as the kernel holds it. The frame says only that it moved, so the
  // answer is read back whole — which is also what puts another window's lines,
  // and the CLI's, in front of this one. The optimistic rows say only what was
  // sent from here, and they do not survive a reload.
  watch([() => toValue(port), () => toValue(moved), () => toValue(sessionState ?? "settled"), () => toValue(sessionPath)], ([p, m, state]) => {
    if (state === "pending" && !m) return;
    let current = true;
    p.queue().then((q) => {
      if (!current) return;
      queue.value = q;
      if (!q.paused) dispatch({ kind: "__queue_resumed" } as never);
    }).catch(() => { if (current) queue.value = null; });
    return () => { current = false; };
  }, { immediate: true });
  const taking = new Set<string>();

  const onQueueRetry = (id: string) => void toValue(port).retryQueued(id).catch(fail);
  const onQueuePause = (on: boolean) => void toValue(port).setQueuePaused(on).catch(fail);
  // A follow-up becomes guidance by being taken back and said again as a steer;
  // the steer the kernel cannot admit is converted to a follow-up, so the line
  // is never lost. The body is read before the entry is given up.
  const onQueueToSteer = async (item: QueueItem) => {
    if (taking.has(item.id)) return;
    taking.add(item.id);
    try {
      const p = toValue(port);
      const text = await p.readQueued(item.id);
      await p.cancelQueued(item.id);
      dispatch({ kind: "__unsent", id: item.id } as never);
      const id = localId();
      dispatch({ kind: "__user", text, pending: true, id } as never);
      const said = await p.steer(text);
      if (said.itemId) dispatch({ kind: "__queued", id, itemId: said.itemId, queued: said.paused || said.disposition === "queued_followup" ? "followup" : "steer", paused: said.paused } as never);
    } catch (e) {
      fail(e);
    } finally {
      taking.delete(item.id);
    }
  };
  // Cancellation lets the dispatcher take the queue head. Accepted guidance
  // must leave the active turn and become a follow-up before that turn ends.
  const onQueueSendNow = async (item: QueueItem) => {
    try {
      const p = toValue(port);
      let itemId = item.id;
      if (item.state === "steer_accepted") {
        const text = await p.readQueued(item.id);
        await p.cancelQueued(item.id);
        dispatch({ kind: "__unsent", id: item.id } as never);
        const id = localId();
        dispatch({ kind: "__user", text, pending: false, id } as never);
        const again = await p.queueFollowup(text);
        itemId = again.itemId;
        dispatch({ kind: "__queued", id, itemId, queued: "followup" } as never);
      }
      await p.moveQueued(itemId, 0);
      await p.cancel();
    } catch (e) {
      fail(e);
    }
  };
  // The panel knows the entry, never the row the composer minted for it, so
  // taking one back here has to name it the way the kernel does. The body is
  // read before the entry is given up: once it is cancelled nothing else holds
  // the text, and a line that cannot be read stays queued rather than lost.
  const onQueueCancel = async (itemId: string) => {
    if (taking.has(itemId)) return;
    taking.add(itemId);
    try {
      const p = toValue(port);
      const text = await p.readQueued(itemId);
      await p.cancelQueued(itemId);
      dispatch({ kind: "__unsent", id: itemId } as never);
      onRestoreText(text);
    } catch (e) {
      taking.delete(itemId);
      fail(e);
    }
  };
  return { queue, restored, onRestoreText, onQueueRetry, onQueuePause, onQueueToSteer, onQueueSendNow, onQueueCancel };
}