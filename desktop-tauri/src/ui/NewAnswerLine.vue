<script setup lang="ts">
import { t } from "../i18n";
</script>

<template>
  <div class="new-answer-line" role="separator" data-new-answer=""><span>{{ t("chat.new_answers_start_here") }}</span></div>
</template>

<script lang="ts">
import { inject, type InjectionKey } from "vue";
import "./NewAnswerLine.css";

export const NewAnswerAnchor: InjectionKey<string | null> = Symbol("NewAnswerAnchor");

export function useNewAnswerAnchor(): string | null {
  return inject(NewAnswerAnchor, null);
}
</script>