import { computed } from "vue";
import { arranged, navLayout, onNavLayoutChange } from "../state/navEntries";
import { useExternal } from "../state/external";
import type { Section } from "./prefsnav";

export type Mark = "chat" | "files" | Section;
export type Destination = [mark: Mark, label: string, section: string, action?: string];

export const DESTINATIONS: Destination[] = [
  ["usage", "app.usage", "usage"],
  ["files", "common.files", "files", "files.open"],
  ["tools", "app.tools_and_permissions", "tools"],
  ["ext", "ext.extension", "ext"],
  ["memory", "data.memory", "memory"],
  ["remote", "settings.remote", "remote"],
];
const KEYS = DESTINATIONS.map(([mark]) => mark);

export function useDestinations() {
  const layout = useExternal(onNavLayoutChange, navLayout);
  return computed(() => arranged(KEYS, layout.value).map((key) => DESTINATIONS.find(([mark]) => mark === key)!));
}
