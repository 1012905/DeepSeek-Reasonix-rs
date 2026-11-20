import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";

/** useCrop answers which focal axis has room to move. A `cover` picture wider
 *  than its box is cropped left and right and nowhere else, so the other axis
 *  cannot move a pixel however far its slider is dragged. It measures the
 *  preview, which is drawn at the window's shape, rather than assuming one.
 *  Both axes count as free until the image decodes: never disable on a guess. */
export function useCrop(url: MaybeRefOrGetter<string | undefined>, kind: MaybeRefOrGetter<"image" | "video" | "scene" | undefined>, box: MaybeRefOrGetter<HTMLElement | null>) {
  const image = ref(0);
  const frame = ref(0);

  watch([() => toValue(url), () => toValue(kind)], (_v, _was, onCleanup) => {
    const src = toValue(url);
    image.value = 0;
    if (!src) return;
    if (toValue(kind) === "video") {
      const clip = document.createElement("video");
      clip.preload = "metadata";
      clip.onloadedmetadata = () => { image.value = clip.videoHeight ? clip.videoWidth / clip.videoHeight : 0; };
      clip.src = src;
      onCleanup(() => {
        clip.onloadedmetadata = null;
        clip.removeAttribute("src");
        clip.load();
      });
      return;
    }
    const img = new Image();
    img.onload = () => { image.value = img.naturalHeight ? img.naturalWidth / img.naturalHeight : 0; };
    img.src = src;
    onCleanup(() => {
      img.onload = null;
    });
  }, { immediate: true });

  watch(() => toValue(box), (el, _was, onCleanup) => {
    if (!el) return;
    const read = () => { frame.value = el.clientHeight ? el.clientWidth / el.clientHeight : 0; };
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    onCleanup(() => ro.disconnect());
  }, { immediate: true, flush: "post" });

  // Ratios a pixel apart leave no room either way, hence the 1% of slack.
  const known = computed(() => image.value > 0 && frame.value > 0);
  const x = computed(() => !known.value || image.value > frame.value * 1.01);
  const y = computed(() => !known.value || image.value < frame.value / 1.01);
  return { known, x, y };
}