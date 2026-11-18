import { computed, onMounted, onUnmounted, ref, toValue, type ComputedRef, type MaybeRefOrGetter } from "vue";
import { navRailMode, onNavRailChange } from "../state/prefs";
import { useExternal } from "../state/external";
import { folded, onFolds } from "./viewport";

/** What the icon column does this render. `drawn` is whether it exists at all:
 *  the machine's choice, unless the window has folded to the scene, where the
 *  workspace rail is a drawer. `shown` is whether it is on screen — in
 *  "collapsed" mode that follows the workspace rail, and the element stays
 *  mounted across that so the rail closing moves it rather than creating it.
 *  The fold is never written back to the choice. */
export function useNavRail(railOpen: MaybeRefOrGetter<boolean>): { drawn: ComputedRef<boolean>; shown: ComputedRef<boolean> } {
  const mode = useExternal(onNavRailChange, navRailMode);
  const phone = ref(folded("scene"));
  let off: (() => void) | undefined;
  onMounted(() => {
    phone.value = folded("scene");
    off = onFolds((now) => { phone.value = now.split(" ").includes("scene"); });
  });
  onUnmounted(() => off?.());
  const drawn = computed(() => mode.value !== "off" && !phone.value);
  const shown = computed(() => drawn.value && (mode.value === "on" || !toValue(railOpen)));
  return { drawn, shown };
}