import { toValue, watch, type MaybeRefOrGetter } from "vue";
import { folded } from "./viewport";

/** Under the scene fold the rail is a drawer laid over the conversation rather
 *  than a column beside it, so going somewhere from it — another session, the
 *  settings — puts it away. Wider, the rail stays where the reader left it. */
export function useDrawerCloses(setRail: (value: boolean) => void, active: MaybeRefOrGetter<string>, settings: MaybeRefOrGetter<unknown>) {
  watch([() => toValue(active), () => toValue(settings)], () => {
    if (folded("scene")) setRail(false);
  }, { immediate: true });
}