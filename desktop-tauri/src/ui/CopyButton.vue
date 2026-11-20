<script lang="ts">
export async function copyText(text: string) {
  if (navigator.clipboard?.writeText) return navigator.clipboard.writeText(text);
  const focused = document.activeElement;
  const carrier = document.createElement("textarea");
  carrier.value = text;
  carrier.readOnly = true;
  carrier.style.cssText = "position:fixed;top:-9999px;opacity:0";
  document.body.append(carrier);
  try {
    carrier.select();
    if (!document.execCommand("copy")) throw new Error("copy rejected");
  } finally {
    carrier.remove();
    if (focused instanceof HTMLElement && focused.isConnected) focused.focus({ preventScroll: true });
  }
}
</script>
<script setup lang="ts">
import { computed, onScopeDispose, ref } from "vue";
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";
import TooltipButton from "./TooltipButton.vue";
const props = withDefaults(defineProps<{
  text: string; iconOnly?: boolean; showFeedback?: boolean; className?: string;
  label?: string; ariaLabel?: string; role?: "menuitem";
}>(), { iconOnly: false, showFeedback: false });
const state = ref<"idle" | "done" | "failed">("idle");
let timer: ReturnType<typeof setTimeout> | undefined;
let disposed = false;
onScopeDispose(() => { disposed = true; clearTimeout(timer); });
async function copy() {
  clearTimeout(timer);
  try {
    await copyText(props.text);
    if (!disposed) state.value = "done";
  } catch {
    if (!disposed) state.value = "failed";
  } finally {
    if (!disposed) timer = setTimeout(() => { state.value = "idle"; }, 1600);
  }
}
const label = computed(() => state.value === "done" ? t("common.copied") : state.value === "failed" ? t("common.copy_failed_try_again") : props.label ?? t("app.copy_response"));
</script>
<template>
  <component :is="iconOnly ? TooltipButton : 'button'" :class="className ? `copy ${className}` : 'copy'"
    type="button" :role="role" :data-icon="iconOnly ? '' : undefined" :data-state="state"
    :data-feedback="showFeedback && state !== 'idle' ? '' : undefined" @click="copy"
    :aria-label="ariaLabel ?? props.label ?? t('app.copy_this_answer')" :title="label">
    <StudioIcon v-if="iconOnly" :name="state === 'done' ? 'check' : state === 'failed' ? 'warning' : 'copy'" />
    <span :class="iconOnly && !(showFeedback && state !== 'idle') ? 'sr-only' : undefined" aria-live="polite">{{ label }}</span>
  </component>
</template>
