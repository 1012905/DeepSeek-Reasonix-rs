import { onWatcherCleanup, ref, toValue, watchEffect, type MaybeRefOrGetter, type Ref } from "vue";

export interface Mark {
  at: number;
  len: number;
}

// The selected state used to be drawn by each item: a tab lit its own underline,
// so switching meant two places changing at once with nothing for the eye to
// follow. One shared marker travels between them instead — the plan cursor has
// always worked this way, and this lifts it out for three more places to share.
// It measures the live element rather than computing a size, because tab widths
// follow the text, the type scale and the window.
export function useMarker(
  host: Ref<HTMLElement | null>,
  sel: MaybeRefOrGetter<string>,
  axis: "x" | "y",
  deps: unknown[],
): Ref<Mark | null> {
  const mark = ref<Mark | null>(null);
  watchEffect(() => {
    const root = host.value;
    const selector = toValue(sel);
    for (const d of deps) toValue(d);
    if (!root) return;
    const measure = () => {
      const el = root.querySelector<HTMLElement>(selector);
      if (!el) {
        mark.value = null;
        return;
      }
      const at = axis === "x" ? el.offsetLeft : el.offsetTop;
      const len = axis === "x" ? el.offsetWidth : el.offsetHeight;
      // Keeping the same object lets Vue skip the trigger. The counts on the
      // tabs change with every streamed item, and without this the marker jitters.
      const prev = mark.value;
      mark.value = prev && prev.at === at && prev.len === len ? prev : { at, len };
    };
    let raf = 0;
    const frame = () => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        measure();
      });
    };
    // Never inside the commit. offsetLeft right after a long transcript mounts
    // forces a layout of the whole document — measured at 133ms a call on a
    // 6x-throttled CPU, over only five calls, because the cost is the forced
    // layout and not the call. On the next frame the browser has laid out
    // anyway and the read is free. A marker one frame late is invisible.
    frame();
    // Observe the target itself, not only the container: a tab is widened by its
    // own text while the container's size holds. ResizeObserver callbacks land at
    // frame end, unlike reading offsetLeft from a dependency, which forces layout.
    //
    // Coalesced to one run per frame. The counts on the tabs change with every
    // item, so switching into a long transcript fires this observer once per
    // mounted block — and each run reads offsetLeft, which is a whole layout on
    // a machine that can least afford one. A marker one frame late is invisible.
    const ro = new ResizeObserver(frame);
    ro.observe(root);
    const el = root.querySelector<HTMLElement>(selector);
    if (el) ro.observe(el);
    onWatcherCleanup(() => {
      if (raf) cancelAnimationFrame(raf);
      ro.disconnect();
    });
  }, { flush: "post" });
  return mark;
}