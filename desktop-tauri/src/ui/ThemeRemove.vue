<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";

/** Deleting a pack from the user's library, asked twice because the files go
 *  with it: the first press arms the button, leaving it or the second press
 *  settles. */
const props = defineProps<{ name: string; onRemove: () => Promise<void> }>();
const phase = ref<"idle" | "confirm" | "pending">("idle");
const press = async () => {
  if (phase.value === "idle") {
    phase.value = "confirm";
    return;
  }
  phase.value = "pending";
  try {
    await props.onRemove();
  } finally {
    phase.value = "idle";
  }
};
const disarm = () => { if (phase.value === "confirm") phase.value = "idle"; };
</script>

<template>
  <button
    type="button"
    class="btn sm pal-rm"
    data-action="theme.remove"
    :data-confirming="phase === 'confirm' ? '' : undefined"
    :aria-label="t('look.delete_theme_name', { name })"
    :disabled="phase === 'pending'"
    :aria-busy="phase === 'pending' || undefined"
    @click="press()"
    @mouseleave="disarm"
    @blur="disarm"
  >
    {{ t(phase === "confirm" ? "common.confirm_delete" : "common.delete") }}
  </button>
</template>