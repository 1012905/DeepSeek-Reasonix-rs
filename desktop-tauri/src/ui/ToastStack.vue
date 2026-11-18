<script lang="ts">
/** The corner takes no choices from the window: it reads state/toasts.ts itself,
 *  so anything that calls pushToast/pushNote shows up here without a prop being
 *  threaded through the tree. onError is for the one thing that can fail inside
 *  the corner — a notice's own button. */
export interface ToastStackProps {
  onError?: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from "vue";
import { t } from "../i18n";
import { MAX_TOASTS, dismissToast, useToasts, type Toast } from "../state/toasts";
import ToastRow from "./ToastRow.vue";

const props = defineProps<ToastStackProps>();

// state/toasts.ts keeps a notice until its reader dismisses it. On this surface
// that would leave a corner full of things already dealt with, so each one is
// given a stay it can be read in: a note is one glance, a failure is read and
// possibly copied, so it stands twice as long. Either way the corner holds
// still while the pointer or the keyboard is on it.
const NOTE_MS = 5_000;
const ERROR_MS = 10_000;

const toasts = useToasts();
const shown = computed(() => toasts.value.slice(-MAX_TOASTS));
const timers = new Map<number, ReturnType<typeof setTimeout>>();
const held = ref(false);

const stay = (toast: Toast) => (toast.tone === "note" ? NOTE_MS : ERROR_MS);

function stop(id: number) {
  const at = timers.get(id);
  if (at === undefined) return;
  clearTimeout(at);
  timers.delete(id);
}

function arm(toast: Toast) {
  if (held.value || timers.has(toast.id)) return;
  timers.set(toast.id, setTimeout(() => {
    timers.delete(toast.id);
    dismissToast(toast.id);
  }, stay(toast)));
}

function release() {
  for (const id of [...timers.keys()]) stop(id);
}

function hold() {
  held.value = true;
  release();
}

function resume() {
  held.value = false;
  for (const toast of shown.value) arm(toast);
}

/** Focus moving between two buttons inside the corner is not focus leaving it,
 *  and treating it as such would restart the clock under the reader's hands. */
function blur(event: FocusEvent) {
  const next = event.relatedTarget as Node | null;
  if (next && (event.currentTarget as HTMLElement).contains(next)) return;
  resume();
}

// Reconciled against what is drawn rather than counted as pushes arrive: a toast
// leaves through its own button, through the ceiling pushing it out, or through
// this timer, and all three have to take their timer with them.
watch(shown, (now) => {
  const live = new Set(now.map((toast) => toast.id));
  for (const id of [...timers.keys()]) if (!live.has(id)) stop(id);
  for (const toast of now) arm(toast);
}, { immediate: true });

onScopeDispose(release);

/** A notice's offer is dismissed first: it is answered, and a button that throws
 *  must not leave the notice it was on sitting there. */
function act(toast: Toast) {
  dismissToast(toast.id);
  try {
    toast.action?.run();
  } catch (e) {
    props.onError?.(e);
  }
}
</script>

<template>
  <div v-if="shown.length > 0" class="toasts" role="region" :aria-label="t('common.notifications')"
    @pointerenter="hold" @pointerleave="resume" @focusin="hold" @focusout="blur">
    <ToastRow v-for="toast in shown" :key="toast.id" :kind="toast.tone === 'note' ? 'note' : 'error'"
      :role="toast.tone === 'note' ? 'status' : 'alert'" :stacked="!!toast.action">
      {{ toast.text }}
      <template #actions>
        <button v-if="toast.action" class="act" data-action="toast.act" :data-target="String(toast.id)"
          @click="act(toast)">{{ toast.action.label }}</button>
        <button class="act ghost" data-action="toast.dismiss" :data-target="String(toast.id)"
          @click="dismissToast(toast.id)">{{ t("common.got_it") }}</button>
      </template>
    </ToastRow>
  </div>
</template>
