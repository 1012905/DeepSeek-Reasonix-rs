<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import type { WorkspaceChange } from "../port/port";
import { workspaceDrag } from "./filedrag";
import { fileKind } from "./fileKind";
import StudioIcon from "./StudioIcon.vue";
import type { TreeRow } from "./workbench_tree";

/** The workspace's files as a tree, each changed one marked with its git status. */
const props = defineProps<{
  rows: TreeRow[];
  collapsed: Set<string>;
  workspaceRoot?: string;
  changes: WorkspaceChange[];
  current: string;
  onFolder: (path: string) => void;
  onFile: (path: string) => void;
  onMenu: (e: MouseEvent, path: string, folder: boolean) => void;
}>();

const status = computed(() => new Map(props.changes.map((c) => [c.path, c.status])));
const folded = ref(false);

const dragAttrs = (path: string) => {
  const d = workspaceDrag(props.workspaceRoot, path);
  return { draggable: d.draggable, onDragstart: d.onDragStart };
};
</script>

<template>
  <div class="workbench-explorer-head workbench-files-head">
    <button type="button" class="workbench-section-fold" data-action="workbench.section-fold" data-target="files" :aria-expanded="!folded"
      :aria-label="t(folded ? 'workbench.expand_section' : 'workbench.collapse_section', { name: t('common.files') })" @click="folded = !folded">
      <StudioIcon :name="folded ? 'chevron' : 'down'" />
    </button>
    <span>{{ t("common.files") }}</span>
  </div>
  <div class="workbench-filelist" :hidden="folded">
    <template v-for="row in rows" :key="row.kind === 'folder' ? `d:${row.path}` : row.path">
      <button
        v-if="row.kind === 'folder'"
        class="workbench-tree-row"
        data-action-click="workbench.folder"
        v-bind="dragAttrs(row.path)"
        :data-target="row.path"
        :style="{ paddingInlineStart: `${8 + row.depth * 14}px` }"
        :aria-expanded="!collapsed.has(row.path)"
        @click="onFolder(row.path)"
        data-action-contextmenu="workbench.menu"
        @contextmenu="onMenu($event, row.path, true)"
      >
        <StudioIcon :name="collapsed.has(row.path) ? 'chevron' : 'down'" />
        <StudioIcon name="folder" />
        <span>{{ row.name }}</span>
      </button>
      <button
        v-else
        class="workbench-tree-row"
        data-action-click="workbench.file"
        v-bind="dragAttrs(row.path)"
        :data-target="row.path"
        :style="{ paddingInlineStart: `${8 + row.depth * 14}px` }"
        :title="row.path"
        :aria-current="current === row.path ? 'page' : undefined"
        @click="onFile(row.path)"
        data-action-contextmenu="workbench.menu"
        @contextmenu="onMenu($event, row.path, false)"
      >
        <span />
        <StudioIcon name="file" :data-kind="fileKind(row.path) || undefined" />
        <span>{{ row.name }}</span>
        <em v-if="status.has(row.path)">{{ status.get(row.path) || "M" }}</em>
      </button>
    </template>
    <p v-if="!rows.length" class="workbench-no-files">{{ t("workbench.no_matching_files") }}</p>
  </div>
</template>