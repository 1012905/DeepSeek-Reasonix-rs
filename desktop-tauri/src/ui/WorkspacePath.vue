<script lang="ts">
import { defineComponent, h } from "vue";

export function compactWorkspaceValue(value: string, length = 38): string {
  const chars = Array.from(value);
  if (chars.length <= length) return value;
  const tail = Math.min(16, Math.floor(length / 2));
  return chars.slice(0, length - tail - 1).join("") + "…" + chars.slice(-tail).join("");
}

export function workspaceName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || path;
}

export const WorkspaceBranchName = defineComponent({
  name: "WorkspaceBranchName",
  props: { value: { type: String, required: true } },
  setup(props) {
    return () => {
      const chars = Array.from(props.value);
      return h("span", { class: "workspace-branch-value", title: props.value },
        chars.length > 32
          ? [h("span", null, chars.slice(0, -12).join("")), h("span", null, chars.slice(-12).join(""))]
          : props.value);
    };
  },
});
</script>

<script setup lang="ts">
import { t } from "../i18n";
import CopyButton from "./CopyButton.vue";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<{ path: string; onResize?: () => void }>();

const parts = props.path.split(/[\\/]/).filter(Boolean);
const short = parts.length > 2 ? "…/" + parts.slice(-2).join("/") : props.path;
</script>

<template>
  <div class="workspace-path">
    <details data-action="workspace.configure" @toggle="onResize">
      <summary :aria-label="t('session.show_full_path')" :title="path"><StudioIcon name="folder" /><span>{{ short }}</span><StudioIcon name="down" /></summary>
      <code>{{ path }}</code>
    </details>
    <CopyButton :text="path" icon-only class="workspace-path-copy" :label="t('session.copy_full_path')" />
  </div>
</template>