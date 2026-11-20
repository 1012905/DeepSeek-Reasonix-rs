import { useExternal } from "./external";
import { onModelFilterChange, readModelFilter } from "./prefs";

export { MODEL_FILTER_KEY, writeModelFilter } from "./prefs";

export function useModelFilter() {
  return useExternal(onModelFilterChange, readModelFilter);
}
