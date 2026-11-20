import { computed, ref, shallowRef, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort } from "../../port/port";
import type { PreviewInfo } from "../../port/workspace";
import { parseGrantedFile } from "../grantedfiles";
import { reason } from "../../i18n/kernel";
import { previewsOffice } from "../../state/prefs";

type Loaded = { info: PreviewInfo; data?: ArrayBuffer; error?: string };
type DocRef = { path: string; root?: string };

export function useDocument(
  port: MaybeRefOrGetter<AgentPort>,
  path: MaybeRefOrGetter<string>,
  shown: MaybeRefOrGetter<boolean>,
  refresh: MaybeRefOrGetter<string>,
) {
  const held = computed(() => parseGrantedFile(toValue(path)));
  const docRef = computed<DocRef>(() => {
    const h = held.value;
    const p = toValue(path);
    return { path: h?.rel ?? p, ...(h ? { root: h.root } : {}) };
  });
  const state = shallowRef<{ ref: DocRef; port: AgentPort; value: Loaded } | undefined>(undefined);
  const reload = ref(0);
  let cache: Loaded | undefined = undefined;
  let lastReload = 0;
  const request = () => (reload.value += 1);

  watch([docRef, () => toValue(port)], () => {
    cache = undefined;
    state.value = undefined;
  }, { immediate: true });

  watch([() => toValue(port), docRef, () => toValue(shown), () => toValue(refresh), reload], ([p, r, sh], _o, onCleanup) => {
    if (!sh) return;
    const controller = new AbortController();
    let busy = false;
    const read = async (force: boolean) => {
      if (busy || controller.signal.aborted) return;
      busy = true;
      let observed: PreviewInfo | undefined;
      try {
        const info = await p.workspacePreviewInfo(r, controller.signal);
        observed = info;
        if (controller.signal.aborted) return;
        let value: Loaded;
        if (info.kind === "unsupported" || info.size > info.limit || (info.kind === "office" && !previewsOffice())) value = { info };
        else if (!force && cache?.data && cache.info.version === info.version) value = { ...cache, info, error: undefined };
        else {
          const loaded = await p.workspacePreviewContent(r, controller.signal);
          value = { info: { ...info, version: loaded.version || info.version }, data: loaded.data };
        }
        if (!controller.signal.aborted) { cache = value; state.value = { ref: r, port: p, value }; }
      } catch (e) {
        if (!controller.signal.aborted) {
          cache = undefined;
          state.value = { ref: r, port: p, value: { info: observed ?? { ref: r, kind: "unsupported", size: 0, limit: 0, version: "", readOnly: true }, error: reason(e) } };
        }
      } finally { busy = false; }
    };
    const force = lastReload !== reload.value;
    lastReload = reload.value;
    void read(force);
    const timer = window.setInterval(() => void read(false), 3000);
    onCleanup(() => { controller.abort(); window.clearInterval(timer); });
  }, { immediate: true });

  const value = computed(() => {
    const s = state.value;
    return s && s.ref === docRef.value && s.port === toValue(port) ? s.value : undefined;
  });
  return { get value() { return value.value; }, reload: request };
}