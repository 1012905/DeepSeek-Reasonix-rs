import { ref, reactive } from "vue";
import { t } from "../i18n";
import type { HostCapabilities, HubPort, TreeWorkspace } from "../port/hub";

export interface Adder {
  add: () => void;
  busy: boolean;
  /** True while the headless fallback is asking for a server-side path. */
  pathOpen: boolean;
  addPath: (path: string) => void;
  closePath: () => void;
}

const LEGACY_CAPABILITIES: HostCapabilities = { pickFolder: true, addWorkspace: true };

/** useAddWorkspace is the one implementation of "open a project", so the
 *  sidebar's entry and the first-run banner cannot drift into two behaviours.
 *  onAdded is what the owner does with the folder once the tree has re-read it. */
export function useAddWorkspace(
  hub: HubPort,
  reload: () => Promise<void>,
  onError: (e: unknown) => void,
  onAdded?: (workspace: TreeWorkspace) => Promise<void>,
): Adder {
  const busy = ref(false);
  const pathOpen = ref(false);

  const add = () => {
    if (busy.value) return;
    busy.value = true;
    void hub
      .hostCapabilities()
      .catch(() => LEGACY_CAPABILITIES)
      .then(async (capabilities) => {
        if (!capabilities.addWorkspace) {
          throw new Error(t("app.this_kernel_does_not_support"));
        }
        if (!capabilities.pickFolder) {
          pathOpen.value = true;
          return;
        }
        const dir = await hub.pickFolder();
        if (dir === null) {
          pathOpen.value = true;
          return;
        }
        // "" is the user closing the panel — an answer, not a reason to ask again.
        if (!dir) return;
        const workspace = await hub.addWorkspace(dir);
        await reload();
        await onAdded?.(workspace);
      })
      .catch(onError)
      .finally(() => (busy.value = false));
  };

  const addPath = (raw: string) => {
    const path = raw.trim();
    if (busy.value || !path) return;
    busy.value = true;
    void hub
      .addWorkspace(path)
      .then(async (workspace) => {
        await reload();
        await onAdded?.(workspace);
      })
      .then(() => (pathOpen.value = false))
      .catch((e) => {
        // The error bar sits above the app; an overlay would hide the reason.
        pathOpen.value = false;
        onError(e);
      })
      .finally(() => (busy.value = false));
  };

  const closePath = () => {
    if (!busy.value) pathOpen.value = false;
  };

  return reactive({ add, busy, pathOpen, addPath, closePath });
}