import { onScopeDispose, toValue, watch, type MaybeRefOrGetter } from "vue";
import { listenAction } from "./listen";
import { pressedChord, typingElsewhere } from "./keys";

/** One window shortcut, named by the action it performs. `fields` says the
 *  chord is also taken from a text field that did not hand it through. */
export interface Shortcut {
  chord: string;
  shift?: boolean;
  fields?: boolean;
  /** Reads the press itself when a chord and a shift flag cannot say which
   *  keys count, as on layouts that type "+" or "0" from other positions. */
  match?: (e: KeyboardEvent) => boolean;
  action: string;
  run: () => void;
}

/** How long a first Escape stays armed for the second that stops the turn. */
export const STOP_ARM_MS = 2500;

type Escape = {
  browser: MaybeRefOrGetter<boolean>;
  settings: MaybeRefOrGetter<boolean>;
  running: MaybeRefOrGetter<boolean>;
  confirm: MaybeRefOrGetter<boolean>;
  closeBrowser: () => void;
  stop: () => void;
  onArm: (armed: boolean) => void;
};

/** The window's keyboard: its shortcut table, and Escape closing the browser
 *  or stopping the turn on screen. With `confirm`, the first Escape only arms
 *  the stop (`onArm` tells the window) and a second within STOP_ARM_MS stops. */
export function useWindowKeys(shortcuts: MaybeRefOrGetter<Shortcut[]>, escape: Escape) {
  let armed: ReturnType<typeof setTimeout> | null = null;
  const disarm = () => {
    if (armed !== null) clearTimeout(armed);
    armed = null;
    escape.onArm(false);
  };
  watch(() => toValue(escape.running), (live) => {
    if (!live) disarm();
  }, { immediate: true });
  const onKey = (e: KeyboardEvent) => {
    // A control that answered the press itself — the code editor's own find
    // on Ctrl+F, or its Escape closing that — has spent it.
    if (e.defaultPrevented) return;
    const table = toValue(shortcuts);
    const hit = table.find((s) => (s.match ? s.match(e) : pressedChord(e, s.chord, !!s.shift)));
    if (hit && (hit.fields || !typingElsewhere(e.target))) {
      e.preventDefault();
      hit.run();
    }
    // Escape stops only the turn on screen, and a pane over it closes first.
    // Anything transient takes the press in the capture phase (useDismiss), so
    // one that reaches here is a press nothing else wanted.
    if (e.key === "Escape" && toValue(escape.browser)) {
      escape.closeBrowser();
    } else if (e.key === "Escape" && !toValue(escape.settings) && toValue(escape.running)) {
      if (!toValue(escape.confirm) || armed !== null) {
        disarm();
        escape.stop();
      } else {
        escape.onArm(true);
        armed = setTimeout(disarm, STOP_ARM_MS);
      }
    }
  };
  let off: (() => void) | undefined;
  watch(
    () => toValue(escape.browser),
    (isBrowser) => {
      off?.();
      off = listenAction(window, "keydown", {
        action: isBrowser ? "browser.open" : "session.stop",
        listener: onKey as EventListener,
      });
    },
    { immediate: true },
  );
  onScopeDispose(() => {
    off?.();
    disarm();
  });
}
