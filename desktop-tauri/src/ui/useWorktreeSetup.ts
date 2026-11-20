import { reason } from "../i18n/kernel";
import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, WorkspaceGit } from "../port/port";
import type { WorktreePreparation, WorktreeSelection, WorktreeSources } from "../port/workspace";

export class WorktreeSetupFailure extends Error {}

interface Attempt {
  id: string;
  port: AgentPort;
  selection: WorktreeSelection;
  session: string;
  status: WorktreePreparation;
  valid: boolean;
  cancelRequested: boolean;
}

const terminal = (phase: string) => ["ready", "failed", "cancelled", "cleanup_failed"].includes(phase);
const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function useWorktreeSetup(port: MaybeRefOrGetter<AgentPort>, fresh: MaybeRefOrGetter<boolean>, root: MaybeRefOrGetter<string>, onChanged: () => void, onTreeChanged?: (git: WorkspaceGit) => void, session: MaybeRefOrGetter<string> = "") {
  const sources = ref<WorktreeSources | null>(null);
  const selection = ref<WorktreeSelection | null>(null);
  const stage = ref<WorktreePreparation | null>(null);
  const setSelection = (value: WorktreeSelection | null) => { selection.value = value; };
  let attempt: Attempt | null = null;
  let prepared = "";
  let mounted = true;
  let sourceRead = 0;
  const refresh = async () => {
    const read = ++sourceRead;
    const value = await toValue(port).worktreeSources();
    if (mounted && read === sourceRead) sources.value = value;
    return value;
  };
  watch(() => toValue(port), () => {
    mounted = true;
    return () => {
      mounted = false;
      const current = attempt;
      if (current) {
        current.valid = false;
        if (!terminal(current.status.phase)) void current.port.cancelWorktreePreparation(current.id).catch(() => {});
      }
    };
  }, { immediate: true });
  watch([() => toValue(port), () => toValue(fresh), () => toValue(root), () => toValue(session)], ([agent, wantFresh, at, held]) => {
    sourceRead++;
    const current = attempt;
    const ownMove = current && at !== current.selection.sourceRoot && at === current.status.workspaceRoot;
    const same = current?.port === agent && ((at === current.selection.sourceRoot && held === current.session) || ownMove);
    if (!same) {
      if (current) {
        current.valid = false;
        if (!terminal(current.status.phase)) void current.port.cancelWorktreePreparation(current.id).catch(() => {});
      }
      attempt = null;
      stage.value = null;
      prepared = "";
      selection.value = null;
      sources.value = null;
    }
    if (wantFresh && !same) void refresh().catch(() => {});
    return () => { sourceRead++; };
  }, { immediate: true });
  const show = (current: Attempt, value: WorktreePreparation) => {
    current.status = value;
    if (mounted && current.valid && attempt === current) stage.value = value;
  };
  const prepare = async (attachments: string[]) => {
    const at = toValue(root);
    const held = toValue(session);
    if (prepared === at && at) return true;
    if (!selection.value && !attempt) return true;
    let current = attempt;
    const reconcile = current && !["failed", "cancelled"].includes(current.status.phase);
    if (!reconcile) {
      if (!selection.value) throw new WorktreeSetupFailure("worktree source is unavailable");
      const id = crypto.randomUUID();
      const agent = toValue(port);
      current = { id, port: agent, selection: attachments.length ? { ...selection.value, attachments } : selection.value, session: held, valid: true, cancelRequested: false,
        status: { id, phase: "starting", sourceRoot: selection.value.sourceRoot, ref: selection.value.ref, startedAt: Date.now() } };
      attempt = current;
    }
    if (!current) return false;
    const active = current;
    show(active, active.status);
    try {
      let value: WorktreePreparation;
      try {
        value = reconcile ? await active.port.worktreePreparation(active.id) : await active.port.prepareWorktree(active.id, active.selection);
      } catch {
        value = await active.port.worktreePreparation(active.id);
      }
      while (active.valid && mounted) {
        show(active, value);
        if (terminal(value.phase)) break;
        await pause(400);
        if (!active.valid || !mounted) return false;
        value = await active.port.worktreePreparation(active.id);
      }
      if (!active.valid || !mounted) return false;
      if (value.phase === "ready" && value.workspaceRoot) {
        prepared = value.workspaceRoot;
        selection.value = null;
        sources.value = null;
        onChanged();
        void active.port.workspaceGit().then(onTreeChanged).catch(() => {});
        if (active.cancelRequested) { show(active, { ...value, phase: "retained" }); return false; }
        await pause(650);
        return active.valid && mounted && !active.cancelRequested;
      }
      if (value.phase === "cancelled") return false;
      throw new WorktreeSetupFailure(value.detail || value.code || "worktree creation failed");
    } catch (error) {
      if (active.valid && mounted && !(error instanceof WorktreeSetupFailure)) {
        show(active, { ...active.status, phase: "unreachable", detail: reason(error) });
      }
      throw error instanceof WorktreeSetupFailure ? error : new WorktreeSetupFailure("worktree status could not be reconciled");
    }
  };
  const cancel = async () => {
    const current = attempt;
    if (!current) return;
    current.cancelRequested = true;
    show(current, { ...current.status, phase: "cancelling" });
    try {
      const value = await current.port.cancelWorktreePreparation(current.id);
      show(current, value.phase === "ready" ? { ...value, phase: "retained" } : value);
    } catch (error) {
      show(current, { ...current.status, phase: "unreachable", detail: reason(error) });
    }
  };
  const dismiss = () => {
    const current = attempt;
    if (current && !["ready", "retained", "failed", "cancelled"].includes(current.status.phase)) return;
    stage.value = null;
    if (current?.status.phase === "failed" || current?.status.phase === "cancelled") attempt = null;
    if (toValue(fresh) && prepared !== toValue(root)) void refresh().catch(() => {});
  };
  return { sources, selection, setSelection, refresh, prepare, stage, cancel, dismiss };
}