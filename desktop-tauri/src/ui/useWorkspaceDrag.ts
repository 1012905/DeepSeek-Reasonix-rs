import { ref, toValue, type MaybeRefOrGetter } from "vue";

const KIND = "application/x-reasonix-workspace";

type Edge = "before" | "after";

/** Reordering the remembered projects by dragging a row. The place a drop names
 *  is a project to land before ("" for the end), so the kernel's order after the
 *  call is the one the person dropped and nothing here keeps an order of its own. */
export function useWorkspaceDrag(order: MaybeRefOrGetter<string[]>, place: (root: string, before: string) => Promise<void>) {
  const dragging = ref("");
  const over = ref<{ root: string; edge: Edge } | null>(null);

  const mine = (e: DragEvent, root: string) => !!dragging.value && toValue(order).includes(root) && [...(e.dataTransfer?.types ?? [])].includes(KIND);
  const edgeOf = (e: DragEvent): Edge => {
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return e.clientY < box.top + box.height / 2 ? "before" : "after";
  };
  const beforeOf = (root: string, edge: Edge) => (edge === "before" ? root : toValue(order)[toValue(order).indexOf(root) + 1] ?? "");

  const start = (e: DragEvent, root: string) => {
    e.dataTransfer!.effectAllowed = "move";
    e.dataTransfer!.setData(KIND, root);
    dragging.value = root;
  };
  const hover = (e: DragEvent, root: string) => {
    if (!mine(e, root)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = "move";
    const edge = edgeOf(e);
    over.value = over.value?.root === root && over.value.edge === edge ? over.value : { root, edge };
  };
  const leave = (root: string) => { over.value = over.value?.root === root ? null : over.value; };
  const end = () => {
    dragging.value = "";
    over.value = null;
  };
  const drop = (e: DragEvent, root: string) => {
    if (!mine(e, root)) return;
    e.preventDefault();
    const list = toValue(order);
    const before = beforeOf(root, edgeOf(e));
    const from = dragging.value;
    end();
    const at = list.indexOf(from);
    const stays = before === from || (before === "" ? at === list.length - 1 : list[at + 1] === before);
    if (!stays) void place(from, before);
  };
  return { dragging, over, start, hover, leave, drop, end };
}