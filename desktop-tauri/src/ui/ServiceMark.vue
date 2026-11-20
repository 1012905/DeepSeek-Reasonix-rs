<script setup lang="ts">
import { computed } from "vue";
import { VendorMark, hasVendorMark } from "./vendormarks.vue";
import { providerMark } from "./modelmenu";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<{ mark?: string; id: string }>();

const slug = computed(() => {
  const mark = props.mark;
  return mark && hasVendorMark(mark) ? mark : "";
});
const tone = computed(() => providerMark(props.id).tone);
const style = computed(() => ({ "--tone": `var(--cat-${tone.value + 1})`, width: "20px", height: "20px" }));
</script>

<template>
  <VendorMark v-if="slug" :slug="slug" :size="20" />
  <i v-else class="mi-mono" :data-tone="tone" :style="style" aria-hidden="true"><StudioIcon name="server" /></i>
</template>
