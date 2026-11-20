<script lang="ts">
export interface PaneTabsProps {
  runtimes: RuntimeView[];
  active: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  /** A tab moved from one place in the strip to another: `to` is the index it
   *  ends up at, after the move, not the slot it was dropped into. */
  onReorder: (from: number, to: number) => void;
  /** Ids of panes whose conversation has something the reader has not seen.
   *  Optional because the mark lives in the session tree, which this strip does
   *  not read: the window derives it (`isUnread` in ui/unread.ts) and hands it
   *  over, so the strip never guesses from a name or a timestamp. */
  unread?: readonly string[];
}
</script>

<script setup lang="ts">
import { computed, onScopeDispose, ref, watch, type CSSProperties, type Ref } from "vue";
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onShowsSessionTabsChange, showsSessionTabs } from "../state/prefs";
import type { RuntimeView } from "../port/hub";
import { useDismiss } from "./dismiss";
import { pinToViewport } from "./place";
import { useMarker, type Mark } from "./marker";
import { arrowTabs } from "./tablist";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<PaneTabsProps>();

// The strip is a local display choice, not a second way to open sessions, so a
// window whose reader turned it off draws nothing here however it is mounted.
const shown = useExternal(onShowsSessionTabsChange, showsSessionTabs);

const bar = ref<HTMLElement | null>(null);
// The travelling underline is an enhancement: without ResizeObserver there is no
// way to know a tab was widened by its own title, and the selected tab's own
// surface is what says which one is in front. Stable per engine, never per render.
const mark: Ref<Mark | null> = typeof ResizeObserver === "undefined"
  ? ref(null)
  : useMarker(bar, '.ptab[aria-selected="true"]', "x", [() => props.active, () => props.runtimes.length]);

/** What a tab says it is. The window's own title for the pane lives in the tree,
 *  which this strip does not read, so it falls back the way the window does: the
 *  pane's name, then the conversation file's own name. */
function labelOf(rt: RuntimeView): string {
  if (rt.name) return rt.name;
  const file = (rt.sessionPath ?? "").replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? "";
  return file.replace(/\.(jsonl|json)$/, "") || t("common.new_session");
}

const unread = computed(() => new Set(props.unread ?? []));

// A tab scrolled out of the strip is a tab you cannot see you are on. Measured
// on the next frame: this fires when a second pane opens, which is also when a
// whole transcript is mounting, and scrollIntoView inside that commit pays for
// a layout of all of it.
let frame = 0;
function followActive(instant = false) {
  if (frame) cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => {
    frame = 0;
    // A shell without scrollIntoView simply leaves the strip where it is.
    bar.value?.querySelector<HTMLElement>('.ptab[aria-selected="true"]')
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: instant ? "instant" : "smooth" });
  });
}
watch([() => props.active, () => props.runtimes.length], () => followActive(), { immediate: true });
onScopeDispose(() => {
  if (frame) cancelAnimationFrame(frame);
});

// A trackpad only ever sends deltaY here, so without this the strip scrolls the
// page instead of itself.
watch(bar, (el, _old, onCleanup) => {
  if (!el) return;
  const onWheel = (event: WheelEvent) => {
    if (event.deltaX !== 0 || el.scrollWidth <= el.clientWidth) return;
    event.preventDefault();
    el.scrollLeft += event.deltaY;
  };
  el.addEventListener("wheel", onWheel, { passive: false });
  onCleanup(() => el.removeEventListener("wheel", onWheel));
}, { immediate: true });

// The overflow menu: what the strip is too narrow to show. Absent engines read
// no ResizeObserver, and then the measurement stands at what it was — the menu
// simply stays as it was drawn rather than disappearing.
const hidden = ref<string[]>([]);
const menuOpen = ref(false);
const menu = ref<HTMLElement | null>(null);

function measure() {
  const el = bar.value;
  if (!el) return;
  if (el.scrollWidth <= el.clientWidth + 1) {
    hidden.value = [];
    return;
  }
  const view = el.getBoundingClientRect();
  const out: string[] = [];
  for (const tab of el.querySelectorAll<HTMLElement>(".ptab[data-pane]")) {
    const box = tab.getBoundingClientRect();
    if (box.left < view.left - 1 || box.right > view.right + 1) out.push(tab.dataset.pane ?? "");
  }
  hidden.value = out.filter(Boolean);
}

let queued = 0;
function remeasure() {
  if (queued) return;
  queued = requestAnimationFrame(() => {
    queued = 0;
    measure();
  });
}
watch(bar, (el, _old, onCleanup) => {
  if (!el) return;
  el.addEventListener("scroll", remeasure, { passive: true });
  let observer: ResizeObserver | null = null;
  if (typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(remeasure);
    observer.observe(el);
  }
  remeasure();
  onCleanup(() => {
    if (queued) cancelAnimationFrame(queued);
    queued = 0;
    el.removeEventListener("scroll", remeasure);
    observer?.disconnect();
  });
}, { immediate: true });
watch(() => props.runtimes.length, remeasure, { flush: "post" });

function openMenu(event: MouseEvent) {
  const at = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuOpen.value = !menuOpen.value;
  if (!menuOpen.value) return;
  void Promise.resolve().then(() => {
    if (menu.value) pinToViewport(menu.value, at.right - menu.value.getBoundingClientRect().width, at.bottom + 6);
  });
}
useDismiss(menuOpen, menu, () => { menuOpen.value = false; });

function pick(id: string) {
  menuOpen.value = false;
  props.onSelect(id);
  followActive(true);
}

// Dragging a tab past another is how the strip is reordered; the kernel's own
// order is untouched, so a reload lands where the panes were opened.
const from = ref<number | null>(null);
const at = ref<number | null>(null);

function dragStart(event: DragEvent, index: number) {
  from.value = index;
  at.value = null;
  if (!event.dataTransfer) return;
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData("text/plain", props.runtimes[index]?.id ?? "");
}
function dragOver(event: DragEvent, index: number) {
  if (from.value === null) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
  at.value = event.clientX > box.left + box.width / 2 ? index + 1 : index;
}
function drop(event: DragEvent) {
  event.preventDefault();
  const start = from.value;
  const slot = at.value;
  from.value = null;
  at.value = null;
  if (start === null || slot === null) return;
  const to = Math.max(0, Math.min(props.runtimes.length - 1, slot > start ? slot - 1 : slot));
  if (to !== start) props.onReorder(start, to);
}
function dragEnd() {
  from.value = null;
  at.value = null;
}

/** Where a dragged tab would land: a bar on the edge it would be inserted at.
 *  A marker of its own rather than a border on the tab, because the tab's border
 *  and box-shadow already say whether it is selected and whether it is focused. */
function dropMark(index: number): CSSProperties | undefined {
  if (at.value !== index && at.value !== index + 1) return undefined;
  const left = at.value === index;
  return {
    position: "absolute",
    top: "4px",
    bottom: "0",
    width: "2px",
    borderRadius: "var(--r-pill)",
    background: "var(--accent-ink)",
    [left ? "left" : "right"]: "-1px",
  };
}
</script>

<template>
  <div v-if="shown && runtimes.length > 0" ref="bar" class="panetabs" role="tablist"
    :aria-label="t('app.session_panes')" @keydown="arrowTabs">
    <div v-for="(rt, index) in runtimes" :key="rt.id" class="ptab" :data-pane="rt.id" :title="rt.root"
      :data-drop="at === index ? 'before' : at === index + 1 ? 'after' : undefined"
      role="tab" draggable="true" :tabindex="rt.id === active ? 0 : -1" :aria-selected="rt.id === active"
      @click="onSelect(rt.id)" @dragstart="dragStart($event, index)" @dragover="dragOver($event, index)"
      @drop="drop($event)" @dragend="dragEnd">
      <span class="ptab-drop" aria-hidden="true" :style="dropMark(index)" />
      <i class="pip" :data-unread="unread.has(rt.id) ? '' : undefined" :role="unread.has(rt.id) ? 'img' : undefined"
        :aria-label="unread.has(rt.id) ? t('common.unread') : undefined" />
      <span class="ptab-nm">{{ labelOf(rt) }}</span>
      <span v-if="rt.host" class="ptab-host"><StudioIcon name="server" />{{ rt.host }}</span>
      <button class="ptab-x" type="button" data-action="pane.close" data-value="one" :data-target="rt.id"
        :title="t('app.close_this_pane')" :aria-label="t('app.close_this_pane')"
        @click.stop="onClose(rt.id)">
        <StudioIcon name="close" />
      </button>
    </div>

    <button class="ptab ptab-new" type="button" data-action="session.new" :title="t('common.start_a_new_session')"
      :aria-label="t('common.start_a_new_session')" @click="onNew()">
      <StudioIcon name="plus" />
    </button>

    <i v-if="mark" class="tabmark" :style="{ width: `${mark.len}px`, transform: `translateX(${mark.at}px)` }" />

    <button v-if="hidden.length > 0" class="ptab ptab-more" type="button" data-action="pane.activate" data-value="overflow"
      aria-haspopup="menu" :aria-expanded="menuOpen" :aria-label="t('common.more')" :title="t('common.more')"
      @click="openMenu($event)">
      +{{ hidden.length }}
      <StudioIcon name="down" />
    </button>

    <Teleport to="body">
      <div v-if="menuOpen" ref="menu" class="tabmenu" role="menu" @click.stop>
        <button v-for="rt in runtimes" :key="rt.id" type="button" role="menuitemradio" :aria-checked="rt.id === active"
          :data-pane="rt.id" @click="pick(rt.id)">
          {{ labelOf(rt) }}
        </button>
      </div>
    </Teleport>
  </div>
</template>
