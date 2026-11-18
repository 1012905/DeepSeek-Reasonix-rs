import { onScopeDispose, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort } from "../port/port";
import type { ExtensionSurface, WireEvent } from "../port/wire";
import type { SessionEvent } from "../state/session";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";

const key = (p: ExtensionSurface) => JSON.stringify([p.pluginId, p.surfaceId]);

export function useSurfaceReplay(port: MaybeRefOrGetter<AgentPort>, dispatch: (event: SessionEvent) => void) {
  const reads = new Set<Set<string>>();
  let epoch = 0;
  let request = 0;
  const observe = (ev: WireEvent) => {
    if (ev.extension) {
      for (const changed of reads) changed.add(key(ev.extension));
    }
  };
  const refresh = async (current: () => boolean = () => true) => {
    const generation = epoch;
    const id = ++request;
    const changed = new Set<string>();
    reads.add(changed);
    try {
      const { surfaces, incomplete } = await toValue(port).extensionSurfaces();
      if (generation !== epoch || id !== request || !current()) return;
      dispatch({ kind: "__extension_snapshot", surfaces: surfaces.filter(p => !changed.has(key(p))) });
      if (incomplete) dispatch({ kind: "__error", text: t("chat.some_extension_surfaces_may_not") });
    } catch (error) {
      if (generation === epoch && id === request && current()) {
        dispatch({ kind: "__error", text: reason(error) });
      }
    } finally {
      reads.delete(changed);
    }
  };
  watch(() => toValue(port), () => {
    void refresh();
    return () => { epoch++; reads.clear(); };
  }, { immediate: true });
  onScopeDispose(() => { epoch++; reads.clear(); });
  return { refresh, observe };
}