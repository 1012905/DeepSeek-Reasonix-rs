// useIntake — the conversation pane is the drop target, not the forty
// pixels of textarea inside it.
//
// Aiming at a one-row input is aiming at 40px, so the whole pane lights up and
// the composer says what letting go will do. Which pane a drop belongs to, and
// whether the payload is a path or bytes, is filedrop.ts's answer; what is left
// here is meaning — and saying it while the drag is still in the air.

import { ref, watch, type Ref } from "vue";
import { useFileDrop, type Dropped } from "./filedrop";
import { PATH_TYPE } from "./filedrag";
import { planIntake, planIsEmpty, planTone, type IntakeItem, type IntakePlan } from "./intake";

interface Options {
  root?: string;
  onReceive: (d: Dropped) => void;
}

export function useIntake({ root, onReceive }: Options): {
  plan: Ref<IntakePlan | null>;
  ref: (node: HTMLElement | null) => () => void;
} {
  const plan = ref<IntakePlan | null>(null);
  // The pane behind the composer, marked so the transcript can recede. Held as
  // a plain field: the zone node is what the drop system hands over, and the
  // pane is that node's ancestor.
  const live = { root, onReceive };
  let pane: HTMLElement | null = null;

  const zone = useFileDrop(
    (d) => {
      plan.value = null;
      live.onReceive(d);
    },
    (over, dt) => { plan.value = over ? preview(dt, live.root) : null; },
  );

  const register = (node: HTMLElement | null) => {
    pane?.removeAttribute("data-intake");
    pane = node?.closest<HTMLElement>(".pane") ?? null;
    // The whole pane takes the drop: aiming at the composer is not something
    // a reader should have to do with a file in hand.
    const off = zone(pane ?? node);
    const held = pane;
    return () => {
      held?.removeAttribute("data-intake");
      pane = null;
      off();
    };
  };

  watch(plan, (shown) => {
    const el = pane;
    if (!el) return;
    if (shown && !planIsEmpty(shown)) el.setAttribute("data-intake", planTone(shown));
    else el.removeAttribute("data-intake");
  });

  return { plan, ref: register };
}

// preview reads what a drag in the air will say: kinds and MIME types, never a
// path and never bytes. Everything it produces is marked pending.
function preview(dt: DataTransfer | null, root?: string): IntakePlan | null {
  if (!dt) return null;
  if (Array.from(dt.types).includes(PATH_TYPE)) return planIntake([{ kind: "file", pending: true }], root);
  const items: IntakeItem[] = [];
  for (const item of Array.from(dt.items ?? [])) {
    if (item.kind === "file") items.push({ kind: "file", mime: item.type, pending: true });
  }
  if (items.length === 0 && carriesText(dt)) items.push({ kind: "text", text: " ", pending: true });
  return planIntake(items, root);
}

function carriesText(dt: DataTransfer): boolean {
  return Array.from(dt.types ?? []).some((type) => type === "text/plain" || type === "text/uri-list");
}
