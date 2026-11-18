import { toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";

export function useSettingLanding(root: Ref<HTMLDivElement | null>, anchor: MaybeRefOrGetter<string>, section: MaybeRefOrGetter<string>, setAnchor: (next: string) => void) {
  watch([root, () => toValue(anchor), () => toValue(section)], (_v, _was, onCleanup) => {
    const wanted = toValue(anchor);
    if (!wanted) return;
    const col = root.value?.querySelector(".prefs-col");
    if (!col) return;
    let target: HTMLElement | null = null;
    let done: ReturnType<typeof setTimeout> | undefined;
    const behavior = matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth";
    const aim = () => {
      const found = col.querySelector<HTMLElement>(`#set-${CSS.escape(wanted)}`);
      if (!found) return;
      for (let parent = found.closest("details"); parent; parent = parent.parentElement?.closest("details") ?? null)
        parent.open = true;
      if (target !== found) {
        target?.removeAttribute("data-landed");
        target = found;
        target.setAttribute("data-landed", "");
        clearTimeout(done);
        done = setTimeout(() => setAnchor(""), 900);
      }
      const panel = col.closest<HTMLElement>(".prefs-main");
      if (panel) {
        const index = panel.querySelector(".page-index")?.getBoundingClientRect().height ?? 0;
        panel.scrollTo({ top: panel.scrollTop + found.getBoundingClientRect().top - panel.getBoundingClientRect().top - index - 16, behavior });
      }
    };
    const grew = new ResizeObserver(aim);
    const arrived = new MutationObserver(aim);
    grew.observe(col);
    arrived.observe(col, { childList: true, subtree: true });
    aim();
    onCleanup(() => {
      clearTimeout(done);
      grew.disconnect();
      arrived.disconnect();
      target?.removeAttribute("data-landed");
    });
  }, { immediate: true, flush: "post" });
}