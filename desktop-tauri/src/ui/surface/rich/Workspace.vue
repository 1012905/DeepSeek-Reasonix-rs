<script lang="ts">
import type { ExtensionViewNode } from "../../../port/wire";

export interface RichInputProps {
  node: ExtensionViewNode;
  value: unknown;
  disabled: boolean;
  change: (value: string | string[]) => void;
}
</script>

<script setup lang="ts">
import { ref, type CSSProperties } from "vue";
import { t } from "../../../i18n";
import "./rich.css";

withDefaults(defineProps<{ title?: string; subtitle: string; badge: string }>(), { title: undefined });
const expanded = ref(false);
const section = ref<HTMLElement | null>(null);

const onKeydown = (e: KeyboardEvent) => {
  if (e.key === "Escape" && expanded.value) {
    e.stopPropagation();
    expanded.value = false;
    return;
  }
  // FocusTrap's active state: Tab stays inside the expanded dialog.
  if (e.key === "Tab" && expanded.value && section.value) {
    const focusable = [...section.value.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])')].filter(el => !el.hasAttribute("disabled"));
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
};

const headingStyle: CSSProperties = { justifyContent: "space-between", alignItems: "flex-start", flexWrap: "nowrap" };
const eyebrowStyle: CSSProperties = { fontSize: "12px", fontWeight: 600, color: "var(--muted)" };
const subtitleStyle: CSSProperties = { fontSize: "14px", color: "var(--muted)" };
</script>

<template>
  <section
    ref="section"
    :class="`rich-workspace${expanded ? ' rich-expanded' : ''}`"
    :role="expanded ? 'dialog' : 'region'"
    :aria-modal="expanded || undefined"
    :aria-label="title || badge"
    tabindex="-1"
    data-action="surface.explore"
    @keydown="onKeydown"
  >
    <div class="rich-workspace-body">
      <div class="xv-stack">
        <div class="rich-heading xv-row" :style="headingStyle">
          <div class="xv-stack">
            <span class="rich-eyebrow" :style="eyebrowStyle">{{ badge }}</span>
            <h3 class="rich-title">{{ title }}</h3>
            <span :style="subtitleStyle">{{ subtitle }}</span>
          </div>
          <button type="button" class="act" data-action="surface.explore" @click="expanded = !expanded">{{ expanded ? t("chat.collapse_workspace") : t("chat.expand_workspace") }}</button>
        </div>
        <slot />
      </div>
    </div>
  </section>
</template>