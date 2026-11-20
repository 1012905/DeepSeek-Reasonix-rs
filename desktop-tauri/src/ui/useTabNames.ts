import { ref } from "vue";
import { keyOf, type Surface } from "./workbench_tabs";

// The names a person gave tabs: held by the tab's stable key for the window's session, and
// stored only when they differ from the page's own label, so leaving the field untouched never
// turns a title that is still changing into a name the person chose.
export function useTabNames(ownLabel: (surface: Surface) => string) {
  const names = ref<Record<string, string>>({});
  const renaming = ref("");
  const setRenaming = (next: string) => { renaming.value = next; };
  const labelFor = (surface: Surface) => names.value[keyOf(surface)] || ownLabel(surface);
  const nameTab = (surface: Surface, value: string) => {
    const key = keyOf(surface);
    const next = value.trim();
    const keep = next !== "" && next !== ownLabel(surface);
    const { [key]: _gone, ...rest } = names.value;
    names.value = keep ? { ...rest, [key]: next } : rest;
    renaming.value = "";
  };
  return { renaming, setRenaming, labelFor, nameTab };
}