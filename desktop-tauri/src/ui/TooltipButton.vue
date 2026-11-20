<script lang="ts">
let dismissActive: (() => void) | undefined;
</script>
<script setup lang="ts">
import { computed, onScopeDispose, ref, useAttrs, useId, watch } from "vue";
import { beside, pinToViewport, useFollow } from "./place";
import { listenAction } from "./listen";
import "./tooltip.css";

defineOptions({ inheritAttrs: false });
const props = defineProps<{ title?: string; shortcut?: string; disabled?: boolean; type?: "button" | "submit" | "reset" }>();
const attrs = useAttrs();
const anchor = ref<HTMLButtonElement | null>(null);
const tip = ref<HTMLDivElement | null>(null);
const open = ref(false);
const id = useId();
let timer: ReturnType<typeof setTimeout> | undefined;
function clear() { clearTimeout(timer); }
function close() {
  clear();
  open.value = false;
  if (dismissActive === close) dismissActive = undefined;
}
function show() {
  clear();
  if (!props.title || props.disabled) return;
  dismissActive?.();
  dismissActive = close;
  open.value = true;
}
function enter(event: PointerEvent) {
  if (event.pointerType !== "touch") { clear(); timer = setTimeout(show, 400); }
}
onScopeDispose(close);
watch(open, (shown, _old, onCleanup) => {
  if (!shown) return;
  const offKey = listenAction(window, "keydown", {
    action: "layer.dismiss",
    listener: ((event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      close();
    }) as EventListener,
  }, true);
  window.addEventListener("blur", close);
  onCleanup(() => { offKey(); window.removeEventListener("blur", close); });
});
function place() {
  if (!anchor.value || !tip.value) return;
  const rect = anchor.value.getBoundingClientRect();
  const box = tip.value.getBoundingClientRect();
  pinToViewport(tip.value, rect.left + (rect.width - box.width) / 2, beside(rect, box.height, innerHeight, "above", 6));
}
useFollow(open, place, () => `${props.title}:${props.shortcut}`);
const describedBy = computed(() => [attrs["aria-describedby"], open.value ? id : undefined].filter(Boolean).join(" ") || undefined);
defineExpose({ focus: (options?: FocusOptions) => anchor.value?.focus(options), anchor });
</script>
<template>
  <button v-bind="attrs" ref="anchor" :type="type ?? 'button'" :disabled="disabled" :aria-describedby="describedBy"
    @pointerenter="enter" @pointerleave="close" @pointerdown="close" @focus="show" @blur="close" @click="close">
    <slot />
  </button>
  <Teleport to="body">
    <div v-if="open && title && !disabled" ref="tip" class="icon-tooltip" :id="id" role="tooltip">
      <span>{{ title }}</span><kbd v-if="shortcut">{{ shortcut }}</kbd>
    </div>
  </Teleport>
</template>
