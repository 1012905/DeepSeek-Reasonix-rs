<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";
import type { RememberedFact } from "../../state/session";
import NoteCard from "./NoteCard.vue";

const ACTIVATION: Record<string, string> = {
  pinned: "chat.remember_pinned",
  relevant: "chat.remember_relevant",
};

const SCOPE: Record<string, string> = { project: "chat.remember_scope_project", global: "chat.remember_scope_global" };

const props = withDefaults(defineProps<{
  m: RememberedFact;
  forgotten?: boolean;
  onForget: (name: string) => void;
}>(), { forgotten: false });

const busy = ref(false);
const forget = () => {
  busy.value = true;
  props.onForget(props.m.name);
};
</script>

<template>
  <NoteCard
    icon="bookmark"
    :gone="forgotten"
    :lead="t(forgotten ? 'chat.forgotten' : 'chat.remembered')"
    :strong="m.title"
    :meta="forgotten ? `· ${t(SCOPE[m.scope] ?? m.scope)}` : `· ${t(SCOPE[m.scope] ?? m.scope)} · ${t(ACTIVATION[m.activation] ?? m.activation)}`"
  >
    <template v-if="!forgotten" #action>
      <button class="note-act" data-action="memory.forget" :data-target="m.name" :disabled="busy" @click="forget">
        {{ t(busy ? "chat.forgetting" : "chat.forget") }}
      </button>
    </template>
    <div v-if="m.description" class="note-text">{{ m.description }}</div>
  </NoteCard>
</template>
