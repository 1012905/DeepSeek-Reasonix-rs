<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { staticDocument } from "./staticDocument";
import { readWorkbook, SHEET_COLUMN_LIMIT, SHEET_ROW_LIMIT, type SheetView } from "./officeSheet";
import { fitZoom, readWordDocument, type WordView } from "./officeWord";
import type { DocumentViewState } from "./viewState";

type Parsed = { kind: "word"; word: WordView } | { kind: "sheets"; sheets: SheetView[] } | { kind: "failed" };

const props = defineProps<{ data: ArrayBuffer; path: string; view: DocumentViewState }>();

const parsed = shallowRef<Parsed | undefined>(undefined);
const sheet = ref(props.view.sheet ?? 0);
const setSheet = (i: number) => { props.view.sheet = i; sheet.value = i; };
const box = ref<HTMLDivElement | null>(null);
const width = ref(0);
const chosen = ref<number | undefined>(props.view.zoom);
const setZoom = (value: number | undefined) => { props.view.zoom = value; chosen.value = value; };

let observer: ResizeObserver | undefined;
onMounted(() => {
  const el = box.value;
  if (!el || typeof ResizeObserver === "undefined") return;
  observer = new ResizeObserver(([entry]) => { width.value = Math.round(entry.contentRect.width); });
  observer.observe(el);
});
onBeforeUnmount(() => observer?.disconnect());

watch([() => props.data, () => props.path], ([data, path], _o, onCleanup) => {
  let live = true;
  parsed.value = undefined;
  const read = /\.xlsx$/i.test(path)
    ? readWorkbook(data).then((sheets): Parsed => ({ kind: "sheets", sheets }))
    : readWordDocument(data).then((word): Parsed => ({ kind: "word", word }));
  read.catch((): Parsed => ({ kind: "failed" })).then((p) => { if (live) parsed.value = p; });
  onCleanup(() => { live = false; });
}, { immediate: true });

const current = computed(() => {
  const p = parsed.value;
  return p?.kind === "sheets" ? p.sheets[Math.min(sheet.value, p.sheets.length - 1)] : undefined;
});
const fit = computed(() => {
  const p = parsed.value;
  return p?.kind === "word" ? fitZoom(width.value, p.word.pageWidth) : 1;
});
const zoom = computed(() => chosen.value ?? fit.value);
const html = computed(() => {
  const p = parsed.value;
  if (p?.kind === "word") return `<style>body{zoom:${zoom.value}}</style>${p.word.html}`;
  return current.value ? `<style>body{zoom:${zoom.value}}</style>${current.value.html}` : undefined;
});
const doc = computed(() => html.value === undefined ? "" : staticDocument(html.value));
const step = (dir: 1 | -1) => setZoom(Math.min(3, Math.max(0.25, Math.round((zoom.value + dir * 0.1) * 100) / 100)));
</script>

<template>
  <div ref="box" class="document-office-view">
    <div v-if="parsed && parsed.kind !== 'failed'" class="office-head">
      <span class="office-ro">{{ t("common.read_only") }}</span>
      <span class="office-note">{{ t("chat.read_only_preview_macros_scripts") }}</span>
      <span class="office-fill" />
      <button type="button" class="pdf-icon" data-action="document.zoom" :data-target="path" :aria-label="t('chat.zoom_out')" :title="t('chat.zoom_out')" @click="step(-1)"><StudioIcon name="minus" /></button>
      <span class="pdf-zoom">{{ Math.round(zoom * 100) }}%</span>
      <button type="button" class="pdf-icon" data-action="document.zoom" :data-target="path" :aria-label="t('chat.zoom_in')" :title="t('chat.zoom_in')" @click="step(1)"><StudioIcon name="plus" /></button>
      <button v-if="parsed.kind === 'word'" type="button" class="pdf-fit" data-action="document.zoom" :data-target="path" :aria-pressed="chosen === undefined" @click="setZoom(undefined)">
        <StudioIcon name="expand" />{{ t("chat.fit_width") }}
      </button>
    </div>
    <p v-if="!parsed" role="status">{{ t("common.loading") }}</p>
    <p v-else-if="parsed.kind === 'failed'" role="alert">{{ t("chat.this_file_could_not_be") }}</p>
    <p v-else-if="parsed.kind === 'sheets' && !current" role="status">{{ t("chat.this_workbook_has_no_visible") }}</p>
    <iframe v-else class="document-html document-office" :title="t('common.office_document_preview')" sandbox="" :srcdoc="doc" />
    <div v-if="parsed?.kind === 'sheets' && current" class="office-sheets">
      <div role="tablist" :aria-label="t('chat.sheets')">
        <button v-for="(s, i) in parsed.sheets" :key="s.name" type="button" role="tab" :aria-selected="s === current" data-action="document.sheet" :data-target="s.name" @click="setSheet(i)">{{ s.name }}</button>
      </div>
      <span class="office-fill" />
      <small v-if="current.clipped">{{ t("chat.showing_the_first_rows_rows", { rows: SHEET_ROW_LIMIT, cols: SHEET_COLUMN_LIMIT }) }}</small>
    </div>
  </div>
</template>