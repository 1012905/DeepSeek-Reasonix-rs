<script setup lang="ts">
import { t } from "../i18n";
import ImagePreview from "./ImagePreview.vue";
import StudioIcon from "./StudioIcon.vue";
import type { Shot } from "./feedbackshots";

withDefaults(defineProps<{ shot: Shot; disabled?: boolean; onRemove: () => void }>(), { disabled: false });
</script>

<template>
  <li>
    <ImagePreview :src="shot.preview" :name="shot.name" class-name="fbk-shot-view">
      <img :src="shot.preview" :alt="shot.name" />
    </ImagePreview>
    <span class="fbk-shot-name">{{ shot.name }}</span>
    <button type="button" class="fbk-shot-x" data-action="feedback.shot.remove" :data-target="shot.id" :disabled="disabled"
      :aria-label="t('feedback.remove_screenshot_name', { name: shot.name })" @click="onRemove">
      <StudioIcon name="close" />
    </button>
  </li>
</template>
