<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import { host } from "../../port/host";
import StudioIcon from "../StudioIcon.vue";

const max = ref(false);
const platform = (): string => document.documentElement.dataset.platform ?? "";
let observer: ResizeObserver | null = null;

function read() {
  void host().isWindowMaximised().then((v) => { max.value = v; });
}

onMounted(() => {
  read();
  // The observer the layout already rides on, so it is the one proven to fire
  // in this shell when the window changes size.
  observer = new ResizeObserver(read);
  observer.observe(document.body);
});

onUnmounted(() => { observer?.disconnect(); observer = null; });
</script>

<template>
  <div v-if="platform() === 'windows'" class="winctl" role="group" :aria-label="t('common.window')">
    <button class="wc" data-action="window.minimize" :aria-label="t('app.minimize')" @click="host().minimiseWindow()">
      <StudioIcon name="minus" />
    </button>
    <button
      class="wc"
      data-action="window.maximize"
      :aria-label="t(max ? 'app.restore' : 'app.maximize')"
      @click="host().toggleMaximiseWindow()"
    >
      <StudioIcon :name="max ? 'copy' : 'stop'" />
    </button>
    <button class="wc close" data-action="window.close" :aria-label="t('common.close')" @click="host().closeWindow()">
      <StudioIcon name="close" />
    </button>
  </div>
</template>
