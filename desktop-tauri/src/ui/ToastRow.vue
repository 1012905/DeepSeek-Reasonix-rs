<script lang="ts">
// The shape of one notice in the window's corner. Exported as a type only: the
// markup below is the single place that decides what a kind looks like.
export type ToastKind = "error" | "note";

export interface ToastRowProps {
  kind: ToastKind;
  /** A failure interrupts a screen reader; a note waits for a pause. */
  role: "alert" | "status";
  /** Wraps the actions under the words when there are too many for one line. */
  stacked?: boolean;
}
</script>

<script setup lang="ts">
import type { StudioIconName } from "./StudioIcon.vue";
import StudioIcon from "./StudioIcon.vue";

defineProps<ToastRowProps>();

const ICON: Record<ToastKind, StudioIconName> = { error: "warning", note: "info" };
const TONE: Record<ToastKind, string> = { error: "warn", note: "ok" };
</script>

<template>
  <div class="toast" :class="kind === 'note' ? 'note' : undefined" :role="role" :data-kind="kind"
    :data-stacked="stacked ? '' : undefined">
    <span class="toast-icon" :data-kind="kind" :data-tone="TONE[kind]">
      <StudioIcon :name="ICON[kind]" />
    </span>
    <div class="toast-body"><slot /></div>
    <div class="toast-actions"><slot name="actions" /></div>
  </div>
</template>
