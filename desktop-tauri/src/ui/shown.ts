import { inject, type InjectionKey } from "vue";
import { useExternal } from "../state/external";

/** Whether the pane this card lives in is the one being looked at. A pane
 *  behind another keeps running but has nobody to animate for. */
export const PaneShown: InjectionKey<boolean> = Symbol("PaneShown");

export const useShown = () => inject(PaneShown, true);

const subscribe = (on: () => void) => {
  document.addEventListener("visibilitychange", on);
  return () => document.removeEventListener("visibilitychange", on);
};

export const usePageVisible = () =>
  useExternal(subscribe, () => document.visibilityState !== "hidden");