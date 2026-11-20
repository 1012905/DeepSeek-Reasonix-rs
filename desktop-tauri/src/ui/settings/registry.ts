import type { Component } from "vue";
import { pageHome, type Section } from "../prefsnav";

/** The contract every view under ./sections follows: it is the file's default
 *  export, and it takes exactly one prop, `settingsProps`, of type
 *  SettingsProps. The sheet hands a section its whole set of props rather than
 *  letting the section reach for ports through injection: the same props answer
 *  for every section, so a missing one is a compile error here, not an empty
 *  page at runtime. Settings.vue renders it as
 *  `<component :is="view" :settings-props="props" />`. */
const VIEWS = import.meta.glob("./sections/*.vue", { eager: true, import: "default" }) as Record<string, Component>;

/** Keyed by file name: ./sections/common.vue answers for the "common" section.
 *  A section with no file stays absent here on purpose — the sheet shows that
 *  as not ported yet instead of guessing at a neighbouring page. */
export const SECTION_VIEW: Partial<Record<Section, Component>> = Object.fromEntries(
  Object.entries(VIEWS).map(([path, view]) => [path.replace(/^.*\//, "").replace(/\.vue$/, ""), view]),
) as Partial<Record<Section, Component>>;

/** The view a section is drawn with: its own, else the page it is folded into
 *  (providers on model, memory on storage, and the rest of FOLDS_INTO). */
export function sectionView(section: Section): Component | undefined {
  return SECTION_VIEW[section] ?? SECTION_VIEW[pageHome(section)];
}
