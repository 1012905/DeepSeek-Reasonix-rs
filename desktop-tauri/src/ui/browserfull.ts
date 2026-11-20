import { computed, ref, toValue, watch, type ComputedRef, type MaybeRefOrGetter } from "vue";
import { pressedChord } from "./keys";
import { listenAction } from "./listen";

type BrowserSize = "split" | "full";

export function useBrowserFull(
  docked: MaybeRefOrGetter<boolean>,
  active: MaybeRefOrGetter<boolean>,
  blocked: MaybeRefOrGetter<boolean> = false,
): { size: ComputedRef<BrowserSize>; toggleFull: () => void } {
  const size = ref<BrowserSize>("split");
  const toggleFull = () => (size.value = size.value === "full" ? "split" : "full");

  watch(() => toValue(docked), (d) => { if (!d) size.value = "split"; });
  watch(() => toValue(blocked), (b) => { if (b) size.value = size.value === "full" ? "split" : size.value; });

  watch([() => toValue(docked), () => toValue(active)], ([d, a], _old, onCleanup) => {
    if (!d || !a) return;
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || e.repeat) return;
      if (pressedChord(e, "f", true)) {
        e.preventDefault();
        toggleFull();
      } else if (e.key === "Escape" && size.value !== "split") {
        e.preventDefault();
        e.stopPropagation();
        size.value = "split";
      }
    };
    // Document bubbling follows controls and dismiss layers, but precedes the
    // window's Escape handler that closes the browser or stops the turn.
    const off = listenAction(document, "keydown", { action: "browser.full", listener: key as EventListener });
    onCleanup(off);
  }, { immediate: true });

  return { size: computed(() => (toValue(docked) ? size.value : "split")), toggleFull };
}