<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { DocumentViewState } from "./viewState";

export interface DocumentPreviewProps {
  port: AgentPort;
  path: string;
  shown: boolean;
  refresh: string;
  view: DocumentViewState;
  onOpenExternal?: () => void;
  onSettings?: (at?: string) => void;
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { t } from "../../i18n";
import { bytes } from "../../i18n/format";
import { useExternal } from "../../state/external";
import { onOfficePreviewChange, previewsOffice } from "../../state/prefs";
import WorkbenchState from "../WorkbenchState.vue";
import StudioIcon from "../StudioIcon.vue";
import { useDocument } from "./useDocument";
import DelimitedPreview from "./DelimitedPreview.vue";
import StaticHtmlPreview from "./StaticHtmlPreview.vue";
import PdfPreview from "./PdfPreview.vue";
import OfficePreview from "./OfficePreview.vue";

const props = defineProps<DocumentPreviewProps>();

const doc = useDocument(() => props.port, () => props.path, () => props.shown, () => props.refresh);
const reload = doc.reload;
const info = computed(() => doc.value?.info);
const data = computed(() => doc.value?.data);
const error = computed(() => doc.value?.error);
const tooLarge = computed(() => !!info.value && info.value.size > info.value.limit);
const text = () => new TextDecoder("utf-8", { fatal: true }).decode(data.value!);

const zoom = ref(props.view.zoom ?? 1);
const setZoom = (next: number) => { zoom.value = next; props.view.zoom = next; };
const pictureUrl = ref("");
const pictureFailed = ref(false);
watch([data, () => props.path], ([buf, path], _o, onCleanup) => {
  pictureFailed.value = false;
  if (!buf || info.value?.kind !== "image") { pictureUrl.value = ""; return; }
  const type = /\.svg$/i.test(path) ? "image/svg+xml" : undefined;
  const url = URL.createObjectURL(new Blob([buf], { type }));
  pictureUrl.value = url;
  onCleanup(() => URL.revokeObjectURL(url));
}, { immediate: true });
onBeforeUnmount(() => { if (pictureUrl.value) URL.revokeObjectURL(pictureUrl.value); });

const officeOn = useExternal(onOfficePreviewChange, previewsOffice);
watch(officeOn, (on) => { if (on && info.value?.kind === "office" && !data.value && !error.value) reload(); });

const external = computed(() => !!props.onOpenExternal && !tooLarge.value && !(info.value?.kind === "office" && !officeOn.value));
</script>

<template>
  <div class="document-preview">
    <div class="document-controls">
      <span v-if="info">{{ info.size.toLocaleString() }} B</span>
      <button type="button" data-action="document.refresh" :data-target="path" @click="reload">{{ t("common.re_freeze") }}</button>
      <button v-if="external" type="button" data-action="workbench.open-file" :data-target="path" @click="onOpenExternal!">
        {{ t("chat.open_in_system_application") }}
      </button>
    </div>

    <p v-if="error" role="alert">{{ error }}</p>
    <p v-else-if="!info" role="status">{{ t("common.loading") }}</p>
    <p v-else-if="info.kind === 'unsupported'" role="alert">{{ t("chat.preview_is_not_available_for") }}</p>
    <div v-else-if="info.kind === 'office' && !officeOn" class="office-off" role="status">
      <p><b>{{ path.split("/").pop() }}</b> · {{ t("chat.office_preview_is_off") }}</p>
      <div v-if="onOpenExternal || onSettings">
        <button v-if="onOpenExternal" type="button" class="office-off-open" data-action="workbench.open-file" :data-target="path" @click="onOpenExternal">{{ t("chat.open_in_system_application") }}</button>
        <button v-if="onSettings" type="button" class="office-off-go" data-action="settings.section" data-value="appearance" @click="onSettings('appearance')">{{ t("chat.turn_preview_on_in_settings") }}</button>
      </div>
    </div>
    <WorkbenchState
      v-else-if="tooLarge"
      icon="layers"
      :title="t('workbench.file_too_large_to_preview')"
      :detail="t('workbench.size_preview_limit', { size: bytes(info.size), limit: bytes(info.limit) })"
      alert
    >
      <button v-if="onOpenExternal" type="button" data-action="workbench.open-file" :data-target="path" @click="onOpenExternal">
        {{ t("chat.open_in_system_application") }}
      </button>
    </WorkbenchState>

    <template v-else-if="data">
      <DelimitedPreview v-if="info.kind === 'table'" :path="path" :text="text()" :view="view" />
      <StaticHtmlPreview v-else-if="info.kind === 'html'" :text="text()" />
      <template v-else-if="info.kind === 'image'">
        <div class="document-controls">
          <button type="button" data-action="document.zoom" :data-target="path" :aria-label="t('chat.zoom_out')" @click="setZoom(Math.max(0.25, zoom - 0.25))">
            <StudioIcon name="zoomOut" />
          </button>
          <button type="button" data-action="document.zoom" :data-target="path" @click="setZoom(1)">{{ Math.round(zoom * 100) }}%</button>
          <button type="button" data-action="document.zoom" :data-target="path" :aria-label="t('chat.zoom_in')" @click="setZoom(Math.min(4, zoom + 0.25))">
            <StudioIcon name="zoomIn" />
          </button>
        </div>
        <p v-if="pictureFailed" role="alert">{{ t("chat.image_unavailable") }}</p>
        <div v-else class="document-picture">
          <img v-if="pictureUrl" :src="pictureUrl" :alt="path" :style="{ width: `${zoom * 100}%`, maxWidth: 'none' }" @error="pictureFailed = true" />
        </div>
      </template>
      <PdfPreview v-else-if="info.kind === 'pdf'" :data="data" :view="view" :on-open-external="onOpenExternal" />
      <OfficePreview v-else-if="info.kind === 'office'" :data="data" :path="path" :view="view" />
      <p v-else role="alert">{{ t("chat.preview_is_not_available_for") }}</p>
    </template>
    <p v-else role="status">{{ t("common.loading") }}</p>
  </div>
</template>