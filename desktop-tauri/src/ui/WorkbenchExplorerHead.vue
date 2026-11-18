<script setup lang="ts">
import { ref, type VNodeChild } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import StudioIcon from "./StudioIcon.vue";

/** The explorer's title row and the actions that act on the whole workspace.
 *  They sit beside the files rather than in settings: this is the one place
 *  the workspace is already what you are looking at. */
const props = defineProps<{
  port: AgentPort;
  count: number;
  revealable: boolean;
  hidden: boolean;
  onHidden: (on: boolean) => void;
  onReveal: () => void;
  openWith?: VNodeChild;
}>();

// Which editor opened it, or why none did. The host knows both and says so,
// because a button that does nothing is indistinguishable from a broken one.
const editorNote = ref("");
const openEditor = () => {
  void props.port
    .openInEditor()
    .then(({ editor }) => (editorNote.value = t("workbench.opened_in_app", { app: editor })))
    .catch((e) => (editorNote.value = reason(e)));
};
</script>

<template>
  <div class="workbench-explorer-head">
    <span>{{ t("workbench.explorer") }}</span>
    <small>{{ count }}</small>
    <button
      class="workbench-open-editor"
      data-action="workbench.hidden"
      :aria-pressed="hidden"
      :title="t('workbench.show_hidden_files')"
      :aria-label="t('workbench.show_hidden_files')"
      @click="onHidden(!hidden)"
    >
      <StudioIcon :name="hidden ? 'eye' : 'eyeoff'" />
    </button>
    <button
      class="workbench-open-editor"
      data-action="workspace.editor"
      :title="editorNote || t('workbench.open_the_workspace_in_a')"
      :aria-label="t('workbench.open_the_workspace_in_a')"
      @click="openEditor"
    >
      <StudioIcon name="code" />
    </button>
    <button
      v-if="revealable"
      class="workbench-open-editor"
      data-action="workbench.reveal"
      :title="t('workbench.show_the_workspace_in_the')"
      :aria-label="t('workbench.show_the_workspace_in_the')"
      @click="onReveal"
    >
      <StudioIcon name="reveal" />
    </button>
    <component :is="openWith" v-if="openWith" />
  </div>
</template>