<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import type { DocumentViewState } from "./viewState";

const props = defineProps<{ text: string; path: string; view: DocumentViewState }>();

const result = ref<{ text: string; rows?: string[][]; error?: boolean } | undefined>(undefined);
const top = ref(props.view.top ?? 0);
const left = ref(props.view.left ?? 0);
const scroll = ref<HTMLDivElement | null>(null);
const height = 30;

watch([result, () => props.view], () => {
  if (scroll.value) { scroll.value.scrollTop = props.view.top ?? 0; scroll.value.scrollLeft = props.view.left ?? 0; }
}, { immediate: true, flush: "post" });

watch([() => props.text, () => props.path], ([text, path], _o, onCleanup) => {
  const worker = new Worker(new URL("./delimited.worker.ts", import.meta.url), { type: "module" });
  const timer = window.setTimeout(() => { worker.terminate(); result.value = { text, error: true }; }, 10_000);
  worker.onmessage = (event: MessageEvent<{ rows?: string[][]; error?: boolean }>) => {
    window.clearTimeout(timer);
    result.value = { text, ...event.data };
    worker.terminate();
  };
  worker.onerror = () => { window.clearTimeout(timer); result.value = { text, error: true }; worker.terminate(); };
  worker.postMessage({ text, delimiter: /\.tsv$/i.test(path) ? "\t" : "," });
  onCleanup(() => { window.clearTimeout(timer); worker.terminate(); });
}, { immediate: true });

const ready = computed(() => result.value?.text === props.text);
const invalid = computed(() => !!result.value && (!!result.value.error || !result.value.rows));
const rows = computed(() => result.value?.rows ?? []);
const start = computed(() => Math.min(Math.max(0, rows.value.length - 1), Math.max(0, Math.floor(top.value / height) - 10)));
const visible = computed(() => rows.value.slice(start.value, start.value + 80));
const columns = computed(() => rows.value.reduce((n, row) => Math.max(n, row.length), 0));
const firstColumn = computed(() => Math.min(Math.max(0, columns.value - 1), Math.max(0, Math.floor(left.value / 160) - 2)));
const count = computed(() => Math.min(20, columns.value - firstColumn.value));

const onScroll = (e: Event) => {
  const el = e.currentTarget as HTMLDivElement;
  props.view.top = el.scrollTop;
  props.view.left = el.scrollLeft;
  top.value = props.view.top;
  left.value = props.view.left;
};
</script>

<template>
  <p v-if="!ready" role="status">{{ t("common.loading") }}</p>
  <p v-else-if="invalid" role="alert">{{ t("chat.the_table_is_invalid_or") }}</p>
  <div v-else ref="scroll" class="document-table" data-action="document.scroll" :data-target="path" @scroll="onScroll">
    <table :aria-label="path" :aria-rowcount="rows.length" :aria-colcount="columns" :style="{ width: `${columns * 160 + 48}px`, tableLayout: 'fixed' }">
      <colgroup>
        <col :style="{ width: '48px' }" />
        <col v-if="firstColumn > 0" :style="{ width: `${firstColumn * 160}px` }" />
        <col v-for="i in count" :key="i" :style="{ width: '160px' }" />
        <col v-if="columns > firstColumn + count" :style="{ width: `${(columns - firstColumn - count) * 160}px` }" />
      </colgroup>
      <tbody>
        <tr aria-hidden="true" :style="{ height: `${start * height}px` }"><td /></tr>
        <tr v-for="(row, i) in visible" :key="start + i" :aria-rowindex="start + i + 1" :style="{ height: `${height}px` }">
          <th scope="row">{{ start + i + 1 }}</th>
          <td v-if="firstColumn > 0" aria-hidden="true" />
          <td v-for="col in count" :key="col" :aria-colindex="firstColumn + col" :title="row[firstColumn + col - 1] ?? ''">{{ row[firstColumn + col - 1] ?? '' }}</td>
          <td v-if="columns > firstColumn + count" aria-hidden="true" />
        </tr>
        <tr aria-hidden="true" :style="{ height: `${Math.max(0, rows.length - start - visible.length) * height}px` }"><td /></tr>
      </tbody>
    </table>
  </div>
</template>