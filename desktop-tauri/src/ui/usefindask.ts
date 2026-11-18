import { ref, toValue, type MaybeRefOrGetter } from "vue";

/** What the window asks of the focused pane's find bar. The chord and the palette
 *  ask it to be open; the title bar button switches it. Under the settings sheet
 *  nothing is in view to close, so there the button only opens, like the rest. */
export function useFindAsk(settings: MaybeRefOrGetter<boolean | string>, leaveSettings: (to: false) => void) {
  const pulse = ref(0);
  const togglePulse = ref(0);
  const open = () => {
    leaveSettings(false);
    pulse.value = pulse.value + 1;
  };
  const toggle = () => {
    if (toValue(settings)) open();
    else togglePulse.value = togglePulse.value + 1;
  };
  return { pulse, togglePulse, open, toggle };
}