<script lang="ts">
import type { JobEntry } from "../../port/port";

/** What a job is called in the list: the name its caller gave it for a person,
 *  else the short preview of what it runs, else its id. */
export const jobName = (j: JobEntry) => j.description?.trim() || j.label || j.id;
</script>

<script setup lang="ts">
import { computed, onWatcherCleanup, ref, useId, watch, type CSSProperties } from "vue";
import { listenAction } from "../listen";

const LONG_PRESS_MS = 450;

/** The name of a job with its full text on hover, focus, Escape to close and a
 *  long press on a touch screen. The tip is real text, so a pointer can move onto
 *  it and select or copy the command (leaving the name for the tip is not
 *  leaving); it is placed in the window because the list sits in a panel that
 *  clips what overflows it. */
const props = defineProps<{ job: JobEntry }>();

const open = ref(false);
const place = ref<CSSProperties>({});
const box = ref<HTMLSpanElement | null>(null);
let press: ReturnType<typeof setTimeout> | null = null;
const id = useId();
const name = computed(() => jobName(props.job));
const command = computed(() => props.job.detail?.trim() || (props.job.description?.trim() ? props.job.label : ""));

watch([() => props.job.id, box], ([jobId, el]) => {
  if (!el) return;
  const stop = () => { if (press) { clearTimeout(press); press = null; } };
  const off = [
    listenAction(el, "keydown", { action: "job.tip", value: jobId, listener: (e) => { if ((e as KeyboardEvent).key === "Escape") { e.stopPropagation(); open.value = false; } } }),
    listenAction(el, "pointerdown", { action: "job.tip", value: jobId, listener: (e) => { if ((e as PointerEvent).pointerType === "touch") { stop(); press = setTimeout(() => { open.value = true; }, LONG_PRESS_MS); } } }),
    listenAction(el, "pointerup", { action: "job.tip", value: jobId, listener: stop }),
    listenAction(el, "pointercancel", { action: "job.tip", value: jobId, listener: stop }),
  ];
  onWatcherCleanup(() => { stop(); off.forEach((f) => f()); });
}, { immediate: true, flush: "post" });

// Under the name, or above it when there is no room below.
watch(open, (isOpen) => {
  if (!isOpen || !box.value) return;
  const r = box.value.getBoundingClientRect();
  const width = Math.min(420, window.innerWidth * 0.8);
  const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
  place.value = r.bottom + 120 > window.innerHeight ? { left, bottom: window.innerHeight - r.top + 4 } : { left, top: r.bottom + 4 };
}, { immediate: true, flush: "post" });

// The teleported tip is not a DOM child of the name, so the pointer crossing
// between the two is checked by hand to keep one hover subtree.
const onLeave = (e: MouseEvent) => {
  const tip = document.getElementById(id);
  if (tip && e.relatedTarget instanceof Node && tip.contains(e.relatedTarget)) return;
  open.value = false;
};
const onTipLeave = (e: MouseEvent) => {
  if (box.value && e.relatedTarget instanceof Node && box.value.contains(e.relatedTarget)) return;
  open.value = false;
};
const onBlur = (e: FocusEvent) => {
  if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) open.value = false;
};
</script>

<template>
  <span
    class="job-name"
    ref="box"
    :aria-describedby="open ? id : undefined"
    @mouseenter="open = true"
    @mouseleave="onLeave"
    @focus="open = true"
    @blur="onBlur"
  >
    <slot />
    <Teleport to="body">
      <span v-if="open" class="job-tip" role="tooltip" :id="id" :style="place" @mouseleave="onTipLeave">
        <span class="jt-name">{{ name }}</span>
        <code v-if="command && command !== name" class="jt-cmd">{{ command }}</code>
      </span>
    </Teleport>
  </span>
</template>