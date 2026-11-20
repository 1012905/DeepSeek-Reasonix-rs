import { nextTick } from "vue";

export const SWAP_MARK = "data-vt";

export function swapping(apply: () => void, kind: string) {
  if (!document.startViewTransition || matchMedia("(prefers-reduced-motion: reduce)").matches) {
    apply();
    return;
  }
  const root = document.documentElement;
  root.setAttribute(SWAP_MARK, kind);
  const transition = document.startViewTransition(async () => {
    apply();
    await nextTick();
  });
  void transition.ready.catch(() => {});
  const finish = () => { if (root.getAttribute(SWAP_MARK) === kind) root.removeAttribute(SWAP_MARK); };
  void transition.finished.then(finish, finish);
}
