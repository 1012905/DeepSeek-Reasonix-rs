<script lang="ts">
export interface SaveNoteState {
  text: string;
  unapplied: boolean;
}
</script>

<script setup lang="ts">
import { t } from "../i18n";

/** The outcome of a refused or half-applied settings save: warn when it was written but not yet in effect, error otherwise. */
defineProps<{ note: SaveNoteState | null }>();
</script>

<template>
  <div v-if="note" class="find" :data-lvl="note.unapplied ? 'warn' : 'err'" :role="note.unapplied ? 'status' : 'alert'">
    <span class="t">{{ t(note.unapplied ? "common.saved_not_applied_yet" : "common.that_did_not_go_through") }}</span>
    <span class="why">{{ note.text }}</span>
  </div>
</template>