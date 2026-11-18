import { onMounted, onUnmounted } from "vue";
import { folded, onFolds } from "./viewport";

// Both side columns share one rule: too narrow to fit and the column folds,
// its seam and grip staying where they are. The choice made at the wide size
// is kept, so narrowing the window and widening it again does not erase a
// column the reader had opened or closed themselves.
export function useFoldAway(name: string, set: (next: boolean | ((cur: boolean) => boolean)) => void, wideDefault = true) {
  let wide = wideDefault;
  let off: (() => void) | undefined;
  onMounted(() => {
    let tight = folded(name);
    off = onFolds((f) => {
      const now = f.split(" ").includes(name);
      if (now === tight) return;
      tight = now;
      set((cur) => {
        if (!now) return wide;
        wide = cur;
        return false;
      });
    });
  });
  onUnmounted(() => off?.());
}