import { shallowRef, toValue, type MaybeRefOrGetter, type ShallowRef } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { HubPort, TreeSession, TreeWorkspace } from "../port/hub";
import type { MoveSessionDialogProps } from "./MoveSessionDialog.vue";

export type MoveDialogState = MoveSessionDialogProps;

interface Inputs {
  hub: HubPort;
  tree: MaybeRefOrGetter<TreeWorkspace[]>;
  reload: () => Promise<unknown> | void;
  label: (session: TreeSession) => string;
  addProject?: () => void;
}

/** Moving a conversation into another project: the request, the dialog that asks
 *  where, and the refusal it shows in the kernel's own words. After a move the
 *  tree is read again rather than patched. */
export function useSessionMoveTo({ hub, tree, reload, label, addProject }: Inputs): {
  dialog: ShallowRef<MoveDialogState | undefined>;
  request: (session: TreeSession, from: string) => void;
  close: () => void;
} {
  const dialog = shallowRef<MoveDialogState | undefined>(undefined);
  let moving: { session: TreeSession; from: string } | null = null;
  let busy = false;
  let error = "";
  let note = "";

  // Every field a render can depend on is rebuilt into one object, so replacing
  // the reference is what tells the dialog its props changed.
  function draw(): void {
    if (!moving) {
      dialog.value = undefined;
      return;
    }
    const { session, from } = moving;
    dialog.value = {
      session,
      label: label(session),
      projects: toValue(tree).filter((w) => w.remembered && w.root !== from),
      busy,
      error,
      note,
      onMove: (target) => { void go(target); },
      onClose: close,
      onAddProject: addProject,
    };
  }

  function close(): void {
    moving = null;
    error = "";
    note = "";
    draw();
  }

  async function go(target: string): Promise<void> {
    if (!moving) return;
    busy = true;
    error = "";
    draw();
    try {
      const moved = await hub.moveSessionTo(moving.session.path, target);
      await reload();
      if (moved.originalRemoved) {
        close();
        return;
      }
      note = t("session.the_conversation_was_moved_but");
      draw();
    } catch (e) {
      error = reason(e);
      draw();
    } finally {
      busy = false;
      draw();
    }
  }

  function request(session: TreeSession, from: string): void {
    error = "";
    note = "";
    moving = { session, from };
    draw();
  }

  return { dialog, request, close };
}
