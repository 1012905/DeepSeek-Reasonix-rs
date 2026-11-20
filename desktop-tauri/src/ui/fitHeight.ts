import { onWatcherCleanup, toValue, watchEffect, type MaybeRefOrGetter, type Ref } from "vue";

// The floor is one line: under an interface zoom scrollHeight is not in the
// units the height written back is, and a smaller number squeezes the box shut.
// A placeholder is not content, so an empty box is one line whatever it wraps to.
export function useFitHeight(box: Ref<HTMLTextAreaElement | null>, text: MaybeRefOrGetter<string>, beforeFit?: (el: HTMLTextAreaElement) => void, room?: (el: HTMLTextAreaElement) => number) {
  const fit = () => {
    const el = box.value;
    if (!el) return;
    beforeFit?.(el);
    const line = parseFloat(getComputedStyle(el).lineHeight) || 22;
    el.style.height = "auto";
    el.style.height = `${toValue(text) ? Math.max(line, el.scrollHeight) : line}px`;
    const left = room?.(el);
    if (left !== undefined && left < el.offsetHeight) el.style.height = `${Math.max(line, left)}px`;
  };

  watchEffect(() => {
    const el = box.value;
    toValue(text);
    if (el) fit();
  }, { flush: "post" });

  watchEffect(() => {
    const el = box.value;
    if (!el || typeof ResizeObserver === "undefined") return;
    let width = el.getBoundingClientRect().width;
    const observer = new ResizeObserver(([entry]) => {
      const next = entry?.contentRect.width ?? width;
      if (Math.abs(next - width) < 0.5) return;
      width = next;
      fit();
    });
    observer.observe(el);
    window.addEventListener("resize", fit);
    onWatcherCleanup(() => {
      observer.disconnect();
      window.removeEventListener("resize", fit);
    });
  }, { flush: "post" });
}