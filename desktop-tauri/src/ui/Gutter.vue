<script lang="ts">
export interface Span {
  min: number;
  max: number;
  def: number;
  key: string;
  // The remembered open width, and the width the column is actually drawn at.
  // They part company while a drag is shutting the panel: the column goes to
  // zero, the remembered width must not.
  css: string;
  col: string;
  // The element the column variable lives on, when it is not the app.
  owner?: string;
}

// A dragged width is the reader's judgement of their own screen, not a layout
// default — so it is stored locally, not in the kernel config: another machine
// or another screen is another judgement.
export const RAIL: Span = { min: 232, max: 440, def: 264, key: "rx-rail-w", css: "--rail-open", col: "--rail-w" };
export const SIDE: Span = { min: 244, max: 540, def: 316, key: "rx-side-w", css: "--side-open", col: "--side-w" };
// The workbench beside the conversation. Wider than the rail because what sits
// in it is a page someone is reading, not a list of names.
export const DOCK: Span = { min: 360, max: 1600, def: 520, key: "rx-dock-w", css: "--dock-open", col: "--dock-w", owner: ".pane" };

const STANDARD_DOCK_MAX = 900;
export const dockMax = (room: number) => Math.min(DOCK.max, Math.max(STANDARD_DOCK_MAX, room / 2));

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

// How far the handle may give at the limit. The width itself stays clamped: the
// constraint is real and should not be faked. What gives is the handle, and it
// says "this is the edge", not "there is more room".
const OVER = 9;
const give = (d: number) => OVER * Math.tanh(d / 90);

// How much further you have to push, past that give, for the panel to close —
// and the same pull the other way to bring it back. The give is what makes the
// threshold safe: a slip stops at the edge, and only sustained force commits.
const COMMIT = 56;

export function widthOf(span: Span): number {
  const saved = Number(localStorage.getItem(span.key));
  return Number.isFinite(saved) && saved > 0 ? clamp(saved, span.min, span.max) : span.def;
}

/** The other half of widthOf: what a released drag commits, written beside the
 *  read so one place spells the key. */
export function keepWidth(span: Span, w: number) {
  localStorage.setItem(span.key, String(Math.round(w)));
}
</script>

<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  edge: "l" | "r";
  span: Span;
  width: number;
  max?: number;
  label: string;
  open: boolean;
  onWidth: (w: number) => void;
  onOpen: (v: boolean) => void;
}>();

const max = computed(() => props.max ?? props.span.max);
let held = 0;

const rail = computed(() => props.edge === "l");

const begin = (bar: HTMLDivElement, pointerId: number, x0: number) => {
  const { span, edge, open, width } = props;
  const maxV = max.value;
  const app = bar.closest(".app") as HTMLElement | null;
  const host = (span.owner ? bar.closest(span.owner) : app) as HTMLElement | null;
  if (!app || !host) return;

  const wasShut = !open;
  let now = open ? width : span.min;
  let shut = wasShut;
  let over = 0;
  let raf = 0;
  bar.setPointerCapture(pointerId);
  bar.dataset.on = "";
  app.dataset.drag = "pointer";

  // Each frame writes the width straight into the variable instead of going
  // through state: the column width is what the dragging hand feels, and a
  // streaming transcript hangs off this tree. It writes the drawn column
  // variable, not the remembered one — the two must part company mid-close.
  const paint = () => {
    host.style.setProperty(span.col, `${shut ? 0 : now}px`);
    bar.style.setProperty("--over", `${over}px`);
    raf = requestAnimationFrame(paint);
  };
  const move = (ev: PointerEvent) => {
    const dx = edge === "l" ? ev.clientX - x0 : x0 - ev.clientX;
    // Positive means widening; the screen direction is applied once at the end
    // so the right column, which widens as the pointer moves left, reads the
    // same way here.
    let pull = 0;
    if (wasShut) {
      shut = dx < COMMIT;
      now = shut ? span.min : clamp(dx, span.min, maxV);
      pull = shut ? give(Math.max(0, dx)) : 0;
    } else {
      const want = width + dx;
      if (want >= span.min) {
        shut = false;
        now = Math.min(maxV, want);
        pull = want > maxV ? give(want - maxV) : 0;
      } else {
        shut = span.min - want > COMMIT;
        now = span.min;
        pull = shut ? 0 : -give(span.min - want);
      }
    }
    over = edge === "l" ? pull : -pull;
  };
  const up = () => {
    cancelAnimationFrame(raf);
    bar.removeEventListener("pointermove", move);
    bar.removeEventListener("pointerup", up);
    bar.removeEventListener("pointercancel", up);
    delete bar.dataset.on;
    bar.style.setProperty("--over", "0px");
    // A frame may not follow the last move, so the column would stop a few
    // pixels from where the hand let go. Those pixels must settle before the
    // tween resumes, or they are walked back slowly — a bounce on release.
    // Reading layout once settles the value with no tween.
    host.style.setProperty(span.col, `${shut ? 0 : now}px`);
    void host.offsetWidth;
    delete app.dataset.drag;
    // Held one frame longer than the release: the stylesheet only produces
    // this same width once Vue has landed the open/closed attribute, and
    // clearing early flashes the column it had before the drag.
    requestAnimationFrame(() => host.style.removeProperty(span.col));
    if (shut === open) props.onOpen(!shut);
    if (!shut) props.onWidth(now);
  };
  bar.addEventListener("pointermove", move);
  bar.addEventListener("pointerup", up);
  bar.addEventListener("pointercancel", up);
  paint();
};

const drag = (e: PointerEvent) => {
  if (e.button !== 0) return;
  e.preventDefault();
  begin(e.currentTarget as HTMLDivElement, e.pointerId, e.clientX);
};

const toggle = () => props.onOpen(!props.open);

const key = (e: KeyboardEvent) => {
  const { span, edge, open, width } = props;
  const maxV = max.value;
  // The pointer commits by pushing through; the keyboard says it outright.
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    toggle();
    return;
  }
  const dir = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
  if (dir) {
    if (!open) return;
    e.preventDefault();
    const bar = e.currentTarget as HTMLDivElement;
    const next = clamp(width + dir * (e.shiftKey ? 48 : 12) * (edge === "l" ? 1 : -1), span.min, maxV);
    // At the limit. The drag has its rubber band; without one here the keyboard
    // just stops responding, which is indistinguishable from a broken key.
    if (next === width) {
      bar.style.setProperty("--bump", `${dir * 5}px`);
      delete bar.dataset.edge;
      void bar.offsetWidth;   // the attribute must leave first, or it will not replay
      bar.dataset.edge = "";
      return;
    }
    // 12px a step against a .34s tween makes a held key chase a rubber band it
    // cannot catch. No tween while the key is held; the tween returns on
    // release — a double-tap reset's whole displacement should still be seen.
    const app = bar.closest(".app") as HTMLElement | null;
    if (app) {
      app.dataset.drag = "key";
      clearTimeout(held);
      held = window.setTimeout(() => delete app.dataset.drag, 220);
    }
    props.onWidth(next);
  } else if (e.key === "Home" && open) {
    e.preventDefault();
    props.onWidth(span.def);
  }
};
</script>

<template>
  <div
    :class="`gutter gutter-${edge}`"
    role="separator"
    aria-orientation="vertical"
    :aria-label="label"
    :aria-valuenow="open ? Math.round(width) : 0"
    :aria-valuemin="0"
    :aria-valuemax="max"
    :data-shut="open ? undefined : ''"
    tabindex="0"
    :data-action-keydown="rail ? 'chrome.rail' : 'browser.open'"
    @pointerdown="drag"
    @dblclick="open && onWidth(span.def)"
    @keydown="key"
  >
  </div>
</template>