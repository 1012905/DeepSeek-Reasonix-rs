import { inject, provide, type InjectionKey } from "vue";
import type { Section } from "../../prefsnav";

/** The section the page around this component is drawing.
 *
 *  Eight sections have two spellings. `FOLDS_INTO` files memory and usage under
 *  storage, versions under account, remote under network, and persona and peers
 *  under session: the index under a page's title promises those anchors, so the
 *  home page owes them somewhere in its column. Each of those also has a file of
 *  its own, because a link — or a setting the model names — opens the sheet at
 *  "memory:memory" or "versions:logs" directly, and the registry answers such
 *  an `at` with the section's own file before falling back to the page the
 *  section folds into.
 *
 *  A block may only be drawn once in a column: two `set-<anchor>` elements make
 *  the landing mark and a search hit a coin toss, and the copy the reader was
 *  not sent to can never be reached while the other slides out from under them.
 *  So a page declares which section it is before it draws anything, and a
 *  section that finds a page of its own name above it draws nothing — the page
 *  above already is that section, and its anchors are already there.
 *
 *  The declaration travels down the column through provide/inject, the shape
 *  `SettingsLeave` already uses: the sheet provides, a section asks, and a
 *  section mounted with no sheet around it still works. It is per column rather
 *  than module-wide, so two windows with a sheet open cannot read each other's
 *  answer, and a replaced column — Settings.vue keys `.prefs-col` on the
 *  section, and replacing that column is what navigation is — starts over. */
const PAGE: InjectionKey<Section> = Symbol("settings.section.page");

/** Declares this file's section as the page its column is drawing.
 *
 *  Called once at the top of a home page's setup, before anything is nested:
 *  the page owns its own blocks, so a section folded into it cannot be drawn
 *  again on top of them. */
export function declarePage(page: Section): void {
  provide(PAGE, page);
}

/** Whether this file's blocks belong in the column above it.
 *
 *  True in both ordinary cases: a folded section drawn as a block on its home
 *  page — which is what makes the page's index land on a row that exists — and
 *  a folded section drawn as the page itself because nothing hosts it, a
 *  section mounted with no sheet around it being a page too. False only when the
 *  page around this component is this very section, and that is the whole of
 *  the guard: the caller renders nothing, so no anchor of its own appears
 *  twice. */
export function drawsHere(own: Section): boolean {
  const host = inject<Section | null>(PAGE, null);
  // Declared either way: whatever this file nests in turn has to be able to
  // tell that a page of this name is already above it.
  provide(PAGE, own);
  return host !== own;
}
