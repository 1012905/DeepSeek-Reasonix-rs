<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../i18n";

const props = withDefaults(defineProps<{
  title: string;
  body: string;
  action: string;
  onAction: () => void;
  waitS?: number;
  retryWhenOnline?: boolean;
  // Not a failure: the dot is the accent colour instead of the error one.
  info?: boolean;
}>(), { waitS: 0, retryWhenOnline: false, info: false });

const left = ref(props.waitS);
watch(() => props.waitS, (waitS) => {
  left.value = waitS;
  if (waitS <= 0) return;
  const timer = setInterval(() => {
    if (left.value <= 1) {
      clearInterval(timer);
      left.value = 0;
    } else {
      left.value -= 1;
    }
  }, 1000);
  return () => clearInterval(timer);
}, { immediate: true });
watch([() => props.retryWhenOnline, () => props.onAction], ([retry, onAction]) => {
  if (!retry) return;
  addEventListener("online", onAction);
  return () => removeEventListener("online", onAction);
}, { immediate: true });
</script>

<template>
  <div :class="info ? 'remote-closed info' : 'remote-closed'" role="alert" :aria-label="title">
    <span aria-hidden="true" />
    <div>
      <b>{{ title }}</b>
      <p>{{ body }}</p>
    </div>
    <button :disabled="left > 0" @click="onAction">
      {{ left > 0 ? t("{action}（{n}）", { action, n: left }) : action }}
    </button>
  </div>
</template>