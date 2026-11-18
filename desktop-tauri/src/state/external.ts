import { onScopeDispose, shallowRef, type ShallowRef } from "vue";

// The Vue answer to an external store: the snapshot is read once per notice,
// and the subscription lives exactly as long as the component scope that asked.
export function useExternal<T>(subscribe: (notify: () => void) => () => void, snapshot: () => T): ShallowRef<T> {
  const value = shallowRef(snapshot()) as ShallowRef<T>;
  const stop = subscribe(() => {
    value.value = snapshot();
  });
  onScopeDispose(stop);
  return value;
}