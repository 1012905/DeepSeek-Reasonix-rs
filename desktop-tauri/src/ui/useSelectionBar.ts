import { onScopeDispose, ref, watch, type Ref } from "vue";
import { listenAction } from "./listen";
import type { SelectionAt } from "./SelectionBar.vue";

function scrollerTop(el: HTMLElement): number {
  for (let at: HTMLElement | null = el; at; at = at.parentElement) {
    if (/auto|scroll/.test(getComputedStyle(at).overflowY)) return at.getBoundingClientRect().top;
  }
  return 0;
}

/** Where the floating bar for a selection inside `box` belongs, once a pointer
 *  selection there has settled; null while there is none. */
export function useSelectionBar(box: Ref<HTMLElement | null>, read: (box: HTMLElement) => string) {
  const at = ref<SelectionAt | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const clear = () => {
    clearTimeout(timer);
    at.value = null;
  };
  watch(at, (isAt, _was, onCleanup) => {
    if (!isAt) return;
    const gone = () => {
      const sel = document.getSelection();
      if (!sel || sel.isCollapsed) clear();
    };
    const stops = [
      listenAction(document, "selectionchange", { action: "reply.selection.bar", listener: gone }),
      listenAction(window, "scroll", { action: "reply.selection.bar", listener: clear }, true),
    ];
    onCleanup(() => stops.forEach((stop) => stop()));
  });
  onScopeDispose(() => clearTimeout(timer));
  const settle = () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const el = box.value;
      const text = el ? read(el) : "";
      if (!text) {
        at.value = null;
        return;
      }
      const range = document.getSelection()!.getRangeAt(0);
      const rect = typeof range.getBoundingClientRect === "function" ? range.getBoundingClientRect() : el!.getBoundingClientRect();
      at.value = { x: rect.left, top: rect.top, bottom: rect.bottom, floor: scrollerTop(el!), text };
    }, 120);
  };
  return { at, settle, clear };
}
