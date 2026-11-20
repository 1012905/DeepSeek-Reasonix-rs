import { computed, toValue, type ComputedRef, type MaybeRefOrGetter } from "vue";
import type { Appearance } from "../port/port";
import { host } from "../port/host";
import { modifierAlone } from "./keys";
import type { Shortcut } from "./windowkeys";
import { stepZoom } from "./zoom";

type Press = { key: string; code: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean };

/** Which way a press moves the interface scale: 1 up, -1 down, 0 reset, null
 *  when it is not one of these chords. The key says it where the layout types
 *  the character, the position where it does not (a French zero is "à"); a
 *  shifted position only counts for the plus, which is a shifted equals. */
export function zoomDirection(e: Press): -1 | 0 | 1 | null {
  if (!modifierAlone(e)) return null;
  if (e.key === "=" || e.key === "+" || e.code === "NumpadAdd" || e.code === "Equal") return 1;
  if (e.key === "-") return -1;
  if (e.key === "0") return 0;
  if (e.shiftKey) return null;
  if (e.code === "Minus" || e.code === "NumpadSubtract") return -1;
  if (e.code === "Digit0" || e.code === "Numpad0") return 0;
  return null;
}

/** The interface scale's keys, through the same save the Appearance control
 *  uses. They exist only inside the desktop shell: in a browser tab the
 *  browser's own zoom answers these chords, and replacing it with a capped one
 *  would take the reader's only way to enlarge the page. Text fields hand them
 *  through because nothing there answers them. */
export function useZoomKeys(look: MaybeRefOrGetter<Appearance>, onLook: (next: Appearance) => void): ComputedRef<Shortcut[]> {
  const step = (dir: -1 | 0 | 1) => {
    const current = toValue(look);
    const next = stepZoom(current.zoom, current.zoomRange, dir);
    if (next !== undefined && next !== (current.zoom || 1)) onLook({ ...current, zoom: next });
  };
  const shell = host().inShell();
  return computed(() =>
    shell
      ? ([1, -1, 0] as const).map((dir) => ({
          chord: "",
          fields: true,
          action: "appearance.zoom",
          match: (e: KeyboardEvent) => zoomDirection(e) === dir,
          run: () => step(dir),
        }))
      : [],
  );
}

