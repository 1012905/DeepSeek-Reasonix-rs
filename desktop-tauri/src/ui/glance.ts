import { onMounted, onUnmounted, ref, type Ref } from "vue";

/** Counts the times the window comes back into view: focused again, or its
 *  page made visible. Disk changes made in another application announce
 *  nothing to this one, and coming back is when somebody can have made one. */
export function useGlance(): Ref<number> {
  const n = ref(0);
  const back = () => {
    if (document.visibilityState !== "hidden") n.value += 1;
  };
  onMounted(() => {
    addEventListener("focus", back);
    document.addEventListener("visibilitychange", back);
  });
  onUnmounted(() => {
    removeEventListener("focus", back);
    document.removeEventListener("visibilitychange", back);
  });
  return n;
}