<script setup lang="ts">
import { computed, type CSSProperties } from "vue";
import { useExternal } from "../../state/external";

function subscribe(change: () => void) {
  const observer = new MutationObserver(change);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => observer.disconnect();
}
const appearance = () => (document.documentElement.dataset.theme === "dark" ? "dark" as const : "light" as const);

const scheme = useExternal(subscribe, appearance);
// The colour-scheme resolver's explicit mappings, kept as the wrapper's own
// variables now that no Mantine theme provides them.
const variables = computed<CSSProperties>(() => (scheme.value === "dark"
  ? {
      "--mantine-color-body": "var(--surface)",
      "--mantine-color-text": "var(--text)",
      "--mantine-color-dimmed": "var(--muted)",
    }
  : {
      "--mantine-color-body": "var(--surface)",
      "--mantine-color-text": "var(--text)",
      "--mantine-color-dimmed": "var(--muted)",
      "--mantine-primary-color-filled": "var(--text)",
      "--mantine-primary-color-filled-hover": "color-mix(in srgb, var(--text) 85%, var(--surface))",
    }));
</script>

<template>
  <div class="surface-kit" :data-mantine-color-scheme="scheme" :style="variables">
    <slot />
  </div>
</template>