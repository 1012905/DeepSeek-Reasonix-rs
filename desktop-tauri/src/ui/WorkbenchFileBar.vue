<script lang="ts">
export type FileMode = "file" | "diff" | "read" | "preview" | "document";
</script>

<script setup lang="ts">
import { type VNodeChild } from "vue";
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";

/** The open file's path, the views it can be seen in, and save. A document that
 *  renders (Markdown, a page) leads with that view and calls the editor
 *  "编辑"; any other file has only its text. */
const props = withDefaults(defineProps<{
  path: string;
  mode: FileMode;
  onMode: (mode: FileMode) => void;
  readable: boolean;
  previewable: boolean;
  canSave: boolean;
  // Set for a file read from a granted folder: the folder, shown as why it cannot be saved.
  readOnlyIn?: string;
  onSave: () => void;
  openWith?: VNodeChild;
  // Opens the file's menu (add to the conversation, copy, reveal) under the "…" button.
  onMore?: (anchor: HTMLElement) => void;
  document?: boolean;
  source?: boolean;
}>(), { document: false, source: true });

const more = (e: MouseEvent) => {
  e.stopPropagation();
  props.onMore?.(e.currentTarget as HTMLElement);
};
</script>

<template>
  <div class="workbench-filebar">
    <strong :title="path">
      <template v-for="(part, i) in path.split('/').slice(0, -1)" :key="i"><span>{{ part }}</span><i aria-hidden="true">/</i></template>
      <b>{{ path.split("/").at(-1) }}</b>
    </strong>
    <div role="group">
      <button v-if="readable" data-action="workbench.mode" data-value="read" :aria-pressed="mode === 'read'" @click="onMode('read')">{{ t("workbench.read") }}</button>
      <button v-if="document" data-action="workbench.mode" data-value="document" :aria-pressed="mode === 'document'" @click="onMode('document')">{{ t("common.preview") }}</button>
      <button v-if="previewable" data-action="workbench.mode" data-value="preview" :aria-pressed="mode === 'preview'" @click="onMode('preview')">{{ t("workbench.run_interactively") }}</button>
      <button v-if="source" data-action="workbench.mode" data-value="file" :aria-pressed="mode === 'file'" @click="onMode('file')">{{ !readOnlyIn && (readable || previewable || document) ? t("common.edit") : t("common.files") }}</button>
      <button v-if="source && !readOnlyIn" data-action="workbench.mode" data-value="diff" :aria-pressed="mode === 'diff'" @click="onMode('diff')">Diff</button>
    </div>
    <component :is="openWith" v-if="openWith" />
    <span v-if="readOnlyIn" class="workbench-readonly" :title="readOnlyIn">{{ t("workbench.read_only_from_a_folder") }}</span>
    <button v-else-if="source" class="workbench-save" data-action="workbench.save" :data-target="path" :disabled="!canSave" @click="onSave">
      <StudioIcon name="check" />
      {{ t("common.save") }}
    </button>
    <button v-if="onMore" class="workbench-reveal" data-action="workbench.file-more" :data-target="path" aria-haspopup="menu" :aria-label="t('common.more')" :title="t('common.more')"
      @click="more">
      <StudioIcon name="more" />
    </button>
  </div>
</template>