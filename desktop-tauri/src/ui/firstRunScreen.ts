/** Which first-run surface the window overlays, from what the machine has
 *  answered. A key still owed is the guide; a machine already configured but
 *  never met gets the welcome scene; everything else is the application. */
export type FirstRunScreen = "guide" | "welcome" | "app";

export interface FirstRunState {
  /** Both reads have answered. Until then the boot screen holds the window. */
  settled: boolean;
  /** The person put the guide off for this launch; it lives in memory only. */
  deferred: boolean;
  /** No usable provider exists, so the key prompt is owed regardless of welcome. */
  setupRequired: boolean;
  /** The welcome scene asked to open the guide anyway. */
  forceGuide: boolean;
  welcomed: boolean;
}

export function firstRunScreen(s: FirstRunState): FirstRunScreen {
  if (!s.settled || s.deferred) return "app";
  if (s.setupRequired || s.forceGuide) return "guide";
  if (!s.welcomed) return "welcome";
  return "app";
}