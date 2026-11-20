import { ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { folded, onFolds } from "./viewport";

/** useFolded is whether the layout has given up what the named fold gives up. */
export function useFolded(name: MaybeRefOrGetter<string>) {
  const now = ref(folded(toValue(name)));
  watch(() => toValue(name), (asked, _was, onCleanup) => {
    now.value = folded(asked);
    onCleanup(onFolds((f) => { now.value = f.split(" ").includes(asked); }));
  }, { immediate: true });
  return now;
}