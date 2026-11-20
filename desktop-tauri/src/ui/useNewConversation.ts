import { toValue, type MaybeRefOrGetter } from "vue";
import type { RuntimeView } from "../port/hub";

export function useNewConversation(active: MaybeRefOrGetter<string>, runtimes: MaybeRefOrGetter<RuntimeView[]>, fallback: MaybeRefOrGetter<string | undefined>,
  open: (req: { root?: string }) => Promise<void>,
  openRemote: (host: string, root?: string) => Promise<void>) {
  return () => {
    const runtime = toValue(runtimes).find((rt) => rt.id === toValue(active));
    return runtime?.host ? openRemote(runtime.host, runtime.root) : open({ root: runtime?.root ?? toValue(fallback) });
  };
}