<script lang="ts">
export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "link";
</script>

<script setup lang="ts">
import { computed, useAttrs } from "vue";

defineOptions({ inheritAttrs: false });

const props = withDefaults(defineProps<{
  variant?: ButtonVariant;
  size?: "sm" | "md";
  iconOnly?: boolean;
}>(), { variant: "secondary", size: "sm", iconOnly: undefined });

const attrs = useAttrs();
const rest = computed(() => {
  const { class: _cls, ...others } = attrs;
  return others;
});
const cls = computed(() => (attrs.class ? `kbtn ${attrs.class}` : "kbtn"));
</script>

<template>
  <button
    type="button"
    v-bind="rest"
    :class="cls"
    :data-variant="props.variant"
    :data-size="props.size"
    :data-icon-only="props.iconOnly || undefined"
  >
    <slot />
  </button>
</template>