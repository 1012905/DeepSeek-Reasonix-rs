import { onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import type { ProviderModelCheckRequest } from "../port/port";
import { checkedFact, type ModelFact, type ModelOrigin } from "./ModelChoice.vue";
import type { Port } from "./Providers.vue";

export function useModelBatch({ port, picked, request, origin, setFacts, draftKey, blocked }: {
  port: MaybeRefOrGetter<Port>;
  picked: MaybeRefOrGetter<string[]>;
  request: MaybeRefOrGetter<Omit<ProviderModelCheckRequest, "model">>;
  origin: ModelOrigin;
  setFacts: (update: (current: Record<string, ModelFact>) => Record<string, ModelFact>) => void;
  draftKey: MaybeRefOrGetter<string>;
  blocked: MaybeRefOrGetter<boolean>;
}): { batch: Ref<"idle" | "running" | "stopped">; testAll: () => Promise<void> } {
  const batch = ref<"idle" | "running" | "stopped">("idle");
  let run: AbortController | null = null;

  const stopBatch = () => {
    if (!run) return false;
    run.abort();
    run = null;
    setFacts((current) => Object.fromEntries(Object.entries(current).map(([model, fact]) => [model, fact.checking ? { ...fact, checking: false } : fact])));
    batch.value = "stopped";
    return true;
  };

  const testAll = async () => {
    if (run || toValue(blocked) || toValue(picked).length === 0) return;
    const ctl = new AbortController();
    run = ctl;
    batch.value = "running";
    const queue = [...toValue(picked)];
    const wire = toValue(port);
    const base = toValue(request);
    const lane = async () => {
      for (let model = queue.shift(); model !== undefined && !ctl.signal.aborted; model = queue.shift()) {
        const m = model;
        setFacts((current) => ({ ...current, [m]: { ...(current[m] ?? { origin }), checking: true } }));
        let result;
        try {
          result = await wire.checkProviderModel({ ...base, model: m });
        } catch {
          result = { model: m, status: "unknown" as const, reason: "network" as const };
        }
        if (ctl.signal.aborted) return;
        setFacts((current) => ({ ...current, [m]: { ...checkedFact(current[m], origin, result), checking: false } }));
      }
    };
    await Promise.all([lane(), lane()]);
    if (run === ctl && !ctl.signal.aborted) {
      run = null;
      batch.value = "idle";
    }
  };

  // A changed draft cancels an in-flight batch: the answers would be about a
  // connection the form no longer describes.
  watch([() => toValue(draftKey), () => toValue(port)], () => {
    if (!stopBatch()) batch.value = batch.value === "stopped" ? "idle" : batch.value;
  });
  onScopeDispose(() => run?.abort());

  return { batch, testAll };
}
