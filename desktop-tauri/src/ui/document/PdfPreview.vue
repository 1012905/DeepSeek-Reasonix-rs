<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { getDocument, GlobalWorkerOptions, TextLayer, type PDFDocumentProxy } from "pdfjs-dist";
import workerURL from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import "pdfjs-dist/web/pdf_viewer.css";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import type { DocumentViewState } from "./viewState";

GlobalWorkerOptions.workerSrc = workerURL;

const STEPS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];

const props = defineProps<{ data: ArrayBuffer; view: DocumentViewState; onOpenExternal?: () => void }>();

const doc = shallowRef<PDFDocumentProxy | undefined>(undefined);
const page = ref(props.view.page ?? 1);
const zoom = ref(props.view.zoom ?? 0);
watch([page, zoom], () => { props.view.page = page.value; props.view.zoom = zoom.value; }, { immediate: true });
const width = ref(600);
// The scale the page was last drawn at, which is what fit-width means in percent.
const drawn = ref(1);
const typed = ref<string | null>(null);
const error = ref(false);
const frame = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const text = ref<HTMLDivElement | null>(null);

let observer: ResizeObserver | undefined;
onMounted(() => {
  if (!frame.value) return;
  observer = new ResizeObserver(([entry]) => { width.value = Math.max(100, entry.contentRect.width - 24); });
  observer.observe(frame.value);
});
onBeforeUnmount(() => observer?.disconnect());

watch(() => props.data, (data, _o, onCleanup) => {
  let live = true;
  doc.value = undefined;
  error.value = false;
  const assets = new URL("./pdfjs/", document.baseURI).href;
  // PDF.js 6 removed eval and isEvalSupported (mozilla/pdf.js@f6bac014).
  // This reader renders only canvas/text; it never creates a scripting manager.
  const task = getDocument({
    data: data.slice(0),
    enableXfa: false,
    cMapUrl: assets + "cmaps/", cMapPacked: true,
    standardFontDataUrl: assets + "standard_fonts/", wasmUrl: assets + "wasm/",
  });
  task.onPassword = () => { if (live) error.value = true; void task.destroy(); };
  void task.promise.then((next) => {
    if (live) { doc.value = next; page.value = Math.min(page.value, next.numPages); }
  }).catch(() => { if (live) error.value = true; });
  onCleanup(() => { live = false; void task.destroy(); });
}, { immediate: true });

watch([doc, page, zoom, width], (_v, _o, onCleanup) => {
  if (!doc.value || !canvas.value || !text.value) return;
  let live = true;
  let cancel = () => {};
  let layer: TextLayer | undefined;
  const surface = canvas.value, overlay = text.value;
  void doc.value.getPage(page.value).then(async (pdfPage) => {
    if (!live) return;
    const base = pdfPage.getViewport({ scale: 1 });
    const scale = zoom.value || width.value / base.width;
    drawn.value = scale;
    const viewport = pdfPage.getViewport({ scale });
    const ratio = Math.min(window.devicePixelRatio || 1, Math.sqrt(16_777_216 / (viewport.width * viewport.height)));
    surface.width = Math.max(1, Math.floor(viewport.width * ratio));
    surface.height = Math.max(1, Math.floor(viewport.height * ratio));
    surface.style.width = `${viewport.width}px`;
    surface.style.height = `${viewport.height}px`;
    overlay.replaceChildren();
    overlay.style.setProperty("--scale-factor", String(scale));
    overlay.style.setProperty("--total-scale-factor", String(scale));
    overlay.style.width = `${viewport.width}px`;
    overlay.style.height = `${viewport.height}px`;
    const render = pdfPage.render({ canvas: surface, viewport, transform: [ratio, 0, 0, ratio, 0, 0] });
    cancel = () => render.cancel();
    layer = new TextLayer({ textContentSource: pdfPage.streamTextContent(), container: overlay, viewport });
    await Promise.all([render.promise, layer.render()]);
  }).catch(() => { if (live) error.value = true; });
  onCleanup(() => { live = false; cancel(); layer?.cancel(); });
}, { flush: "post" });

const pages = computed(() => doc.value?.numPages ?? 0);
const go = (raw: string) => {
  typed.value = null;
  const n = Number.parseInt(raw, 10);
  if (Number.isFinite(n) && pages.value) page.value = Math.min(pages.value, Math.max(1, n));
};
const step = (dir: 1 | -1) => {
  const next = dir > 0 ? STEPS.find((n) => n > drawn.value + 0.01) : [...STEPS].reverse().find((n) => n < drawn.value - 0.01);
  zoom.value = next ?? (dir > 0 ? STEPS.at(-1)! : STEPS[0]);
};
const onType = (e: Event) => { typed.value = (e.target as HTMLInputElement).value; };
</script>

<template>
  <div v-if="error" class="pdf-fail" role="alert">
    <StudioIcon name="warning" />
    <span>{{ t("chat.cannot_display_this_pdf_it") }}</span>
    <button v-if="onOpenExternal" type="button" data-action="workbench.open-file" @click="onOpenExternal">{{ t("chat.open_in_system_application") }}</button>
  </div>
  <div v-else class="document-pdf">
    <div class="pdf-bar">
      <button type="button" class="pdf-icon" data-action="document.page" :disabled="page <= 1 || !doc" :aria-label="t('common.previous_page')" :title="t('common.previous_page')"
        @click="page -= 1"><StudioIcon name="chevron" class="pdf-back" /></button>
      <span class="pdf-count">
        <input data-action-keydown="document.page" data-action-change="document.page" data-action-blur="document.page" inputmode="numeric" :aria-label="t('chat.page_number')" :disabled="!doc"
          :value="typed ?? String(page)" @input="onType"
          @keydown.enter="go(($event.target as HTMLInputElement).value)" @keydown.escape="typed = null"
          @blur="go(($event.target as HTMLInputElement).value)" />
        / {{ pages || "—" }}
      </span>
      <button type="button" class="pdf-icon" data-action="document.page" :disabled="!doc || page >= pages" :aria-label="t('common.next_page')" :title="t('common.next_page')"
        @click="page += 1"><StudioIcon name="chevron" /></button>
      <span class="pdf-fill" />
      <button type="button" class="pdf-icon" data-action="document.zoom" :disabled="!doc" :aria-label="t('chat.zoom_out')" :title="t('chat.zoom_out')" @click="step(-1)"><StudioIcon name="minus" /></button>
      <span class="pdf-zoom">{{ Math.round(drawn * 100) }}%</span>
      <button type="button" class="pdf-icon" data-action="document.zoom" :disabled="!doc" :aria-label="t('chat.zoom_in')" :title="t('chat.zoom_in')" @click="step(1)"><StudioIcon name="plus" /></button>
      <button type="button" class="pdf-fit" data-action="document.zoom" :aria-pressed="zoom === 0" :disabled="!doc" @click="zoom = 0">
        <StudioIcon name="expand" />{{ t("chat.fit_width") }}
      </button>
    </div>
    <p v-if="!doc" role="status">{{ t("common.loading") }}</p>
    <div ref="frame" class="document-pdf-scroll">
      <div class="document-pdf-page" :hidden="!doc">
        <canvas ref="canvas" /><div ref="text" class="textLayer" />
      </div>
    </div>
  </div>
</template>