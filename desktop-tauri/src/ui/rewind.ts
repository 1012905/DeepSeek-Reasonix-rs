import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { reason } from "../i18n/kernel";
import { t } from "../i18n";
import type { AgentPort, RewindResult, RewindScope } from "../port/port";

export type ReloadFailure = { error: string; working: boolean };
export type RestoreNotice = { tx: string; files: number; working: boolean; error: string };

interface Owner {
  active: boolean;
  epoch: number;
  run: number;
  notice: RestoreNotice | null;
  commit: { planId: string; promise: Promise<RewindResult> } | null;
  undo: Promise<void> | null;
  reload: Promise<void> | null;
}

const makeOwner = (): Owner => ({ active: true, epoch: 0, run: 0, notice: null, commit: null, undo: null, reload: null });

/** A restore receipt belongs to the conversation, not to a transcript card that
 *  the restore itself can remove. Session changes retire pending callbacks too. */
export function useRewindActions(
  port: MaybeRefOrGetter<AgentPort>,
  reloadSession: (current?: () => boolean) => void | Promise<void>,
  onRestoreText: (text: string) => void,
  session: MaybeRefOrGetter<string> = "",
  running: MaybeRefOrGetter<boolean> = false,
) {
  const owner = ref<Owner>(makeOwner());
  const notice = ref<{ owner: Owner; value: RestoreNotice | null } | null>(null);
  const reloadFailure = ref<(ReloadFailure & { owner: Owner }) | null>(null);

  watch(owner, (o) => {
    o.active = true;
    return () => { o.active = false; o.epoch++; };
  }, { immediate: true });
  watch([() => toValue(port), () => toValue(session)], () => {
    owner.value = makeOwner();
  });
  const publish = (o: Owner, value: RestoreNotice | null) => {
    o.notice = value;
    notice.value = { owner: o, value };
  };
  // A new run consumes kernel undo, but a completed mutation still needs its history read.
  watch([owner, () => toValue(running)], ([o, run]) => {
    if (run) { o.run++; publish(o, null); }
  }, { immediate: true });
  const onReloadSession = (): Promise<void> => {
    const o = owner.value;
    if (!o.active) return Promise.resolve();
    if (o.reload) return o.reload;
    const epoch = o.epoch;
    const current = () => o.active && o.epoch === epoch;
    const prev = reloadFailure.value;
    reloadFailure.value = prev?.owner === o ? { ...prev, working: true } : prev;
    const operation = (async () => {
      try {
        await reloadSession(current);
        if (current()) reloadFailure.value = null;
      } catch (e) {
        if (current()) reloadFailure.value = { owner: o, error: reason(e), working: false };
      }
    })();
    o.reload = operation;
    void operation.finally(() => { if (o.reload === operation) o.reload = null; });
    return operation;
  };
  const onPrepareRewind = async (turn: number, scope: RewindScope) => {
    const o = owner.value;
    if (!o.active || o.commit || o.undo || o.reload) throw new Error(t("common.restoring"));
    const epoch = o.epoch;
    const run = o.run;
    const plan = await toValue(port).prepareRewind(turn, scope);
    if (!o.active || o.epoch !== epoch || o.run !== run) throw new Error(t("app.the_conversation_changed_choose_a"));
    return plan;
  };
  const onCommitRewind = (planId: string, text?: string): Promise<RewindResult> => {
    const o = owner.value;
    if (!o.active) return Promise.reject(new Error(t("app.the_conversation_changed_choose_a")));
    if (o.commit) return o.commit.planId === planId ? o.commit.promise : Promise.reject(new Error(t("common.restoring")));
    if (o.undo || o.reload) return Promise.reject(new Error(t("common.restoring")));
    const epoch = o.epoch;
    const run = o.run;
    const previous = o.notice;
    if (previous) publish(o, { ...previous, working: true, error: "" });
    const operation = (async () => {
      const result = await toValue(port).commitRewind(planId);
      if (!o.active || o.epoch !== epoch) return result;
      const tx = result.undoAvailable ? result.transactionId : undefined;
      if (o.run === run) {
        publish(o, tx ? { tx, files: (result.written?.length ?? 0) + (result.deleted?.length ?? 0), working: true, error: "" } : null);
        if (result.conversationOk && text !== undefined) onRestoreText(text);
      }
      await onReloadSession();
      if (o.active && o.epoch === epoch && tx && o.notice && o.notice.tx === tx) publish(o, { ...o.notice, working: false });
      return result;
    })();
    o.commit = { planId, promise: operation };
    void operation.catch(() => {
      if (o.active && o.epoch === epoch && o.run === run) publish(o, previous);
    }).finally(() => {
      if (o.commit?.promise === operation) o.commit = null;
    });
    return operation;
  };
  const onUndoRewind = (transactionId: string): Promise<void> => {
    const o = owner.value;
    if (!o.active || o.notice?.tx !== transactionId) return Promise.resolve();
    if (o.undo) return o.undo;
    if (o.commit || o.reload) return Promise.resolve();
    const receipt = o.notice;
    const epoch = o.epoch;
    publish(o, { ...receipt, working: true, error: "" });
    const operation = (async () => {
      try {
        await toValue(port).undoRewind(transactionId);
        if (!o.active || o.epoch !== epoch) return;
        if (o.notice?.tx === transactionId) publish(o, null);
        await onReloadSession();
      } catch (e) {
        if (o.active && o.epoch === epoch && o.notice?.tx === transactionId) {
          publish(o, { ...receipt, working: false, error: reason(e) });
        }
      }
    })();
    o.undo = operation;
    void operation.finally(() => { if (o.undo === operation) o.undo = null; });
    return operation;
  };
  const onUndoAvailable = async (transactionId: string) => {
    const o = owner.value;
    if (!o.active) throw new Error(t("app.the_conversation_changed_choose_a"));
    if (o.commit || o.reload || (o.undo && o.notice?.tx !== transactionId)) throw new Error(t("common.restoring"));
    // Backend offers survive reloads that retired the local restore receipt.
    if (o.notice?.tx !== transactionId) publish(o, { tx: transactionId, files: 0, working: false, error: "" });
    await onUndoRewind(transactionId);
    if (o.notice?.tx === transactionId && o.notice.error) throw new Error(o.notice.error);
  };
  const dismissRestore = () => { const o = owner.value; if (!o.notice?.working) publish(o, null); };
  const onPrepareFileRevert = (path: string) => toValue(port).prepareFileRevert(path);
  const onCommitFileRevert = async (planId: string, resolution?: string) => {
    const o = owner.value;
    const epoch = o.epoch;
    const result = await toValue(port).commitFileRevert(planId, resolution);
    if (o.active && o.epoch === epoch && result.transactionId) publish(o, null);
    return result;
  };
  return {
    onUndoAvailable,
    reloadFailure: computed(() => (reloadFailure.value?.owner === owner.value ? reloadFailure.value : null)),
    onReloadSession,
    restoreNotice: computed(() => (notice.value?.owner === owner.value ? notice.value.value : null)),
    dismissRestore,
    onPrepareRewind,
    onCommitRewind,
    onUndoRewind,
    onPrepareFileRevert,
    onCommitFileRevert,
  };
}