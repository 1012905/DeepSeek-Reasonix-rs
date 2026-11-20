<script setup lang="ts">
import { computed, ref, toRef } from "vue";
import { useDismiss } from "../dismiss";
import { beside, pinToViewport, useFollow, zoom } from "../place";

// Gap between the trigger and the list, in the space a rect reports.
const GAP = 5;

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The list one of a finished reply's action-row buttons opens. It is drawn
 *  into the body because the card paints under containment inside a scroller,
 *  and either one cuts off a list positioned within it. `anchor` is the
 *  trigger, which stays in the row. */
const props = defineProps<{
  anchor: HTMLButtonElement | null;
  open: boolean;
  onClose: () => void;
}>();

const openRef = computed(() => props.open);
const anchorRef = toRef(props, "anchor");
const pop = ref<HTMLDivElement | null>(null);
const entry = ref<HTMLSpanElement | null>(null);

// A list that closes while it holds the focus hands it back to its trigger,
// or the caret drops to the document and the next Tab starts from the top.
const close = () => {
  if (pop.value?.contains(document.activeElement)) props.anchor?.focus();
  props.onClose();
};
useDismiss(openRef, anchorRef, close, pop);

// Above the row when the whole list fits there, below it otherwise. The layout
// height, not the painted one, so the entrance animation cannot skew it.
const place = () => {
  const el = pop.value;
  const to = props.anchor?.getBoundingClientRect();
  if (!el || !to) return;
  pinToViewport(el, to.left, beside(to, el.offsetHeight * zoom(), innerHeight, "above", GAP));
};
useFollow(openRef, place, undefined, { anchor: anchorRef, container: pop });

// The list left the row's DOM, so the row keeps a stop where it was: Tab from
// the trigger enters at the first item, Shift+Tab from the next control at
// the last, and walking off either end lands where the row would have.
const rows = () => [...(pop.value?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
const enter = (e: FocusEvent) => {
  const from = e.relatedTarget;
  const back = from instanceof Node && !!entry.value && !!(entry.value.compareDocumentPosition(from) & Node.DOCUMENT_POSITION_FOLLOWING);
  const all = rows();
  (back ? all[all.length - 1] : all[0])?.focus();
};
const leave = () => {
  const order = [...document.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !pop.value?.contains(el));
  const next = entry.value ? order[order.indexOf(entry.value) + 1] : undefined;
  (next ?? props.anchor)?.focus();
};
const focusAnchor = () => props.anchor?.focus();
</script>

<template>
  <template v-if="open">
    <span ref="entry" class="sr-only" tabindex="0" @focus="enter" />
    <Teleport to="body">
      <div ref="pop" class="acts-pop" role="menu">
        <span class="sr-only" tabindex="0" @focus="focusAnchor" />
        <slot />
        <span class="sr-only" tabindex="0" @focus="leave" />
      </div>
    </Teleport>
  </template>
</template>
