<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import type { PickedElement } from "../port/host";
import type { SourceEdit, StyleProperty } from "./htmlsource";
import type { PageEditFailure, PageEditor } from "./pageedit";

const props = defineProps<{
  element: PickedElement;
  /** The disk path of the file the page was opened from, or "" for any other page. */
  filePath: string;
  editor?: PageEditor;
  onSaved: () => void;
  onClose: () => void;
}>();

const FAILED: Record<PageEditFailure, () => string> = {
  not_found: () => t("workbench.edit_not_found"),
  ambiguous: () => t("workbench.edit_ambiguous"),
  unmappable: () => t("workbench.edit_unmappable"),
  mismatch: () => t("workbench.edit_mismatch"),
  not_plain_text: () => t("workbench.edit_not_plain_text"),
  bad_value: () => t("workbench.edit_bad_value"),
  outside: () => t("workbench.edit_outside"),
  conflict: () => t("workbench.edit_conflict"),
  failed: () => t("workbench.edit_failed"),
};

const STYLES: { property: StyleProperty; label: () => string }[] = [
  { property: "color", label: () => t("workbench.edit_color") },
  { property: "backgroundColor", label: () => t("workbench.edit_background") },
  { property: "fontSize", label: () => t("workbench.edit_font_size") },
];

// What the file holds now: a save moves it, so the next one is measured from there.
const element = ref<PickedElement>(props.element);
const text = ref(props.element.text ?? "");
const styles = ref<Partial<Record<StyleProperty, string>>>(props.element.style ?? {});
const busy = ref(false);
const note = ref<{ kind: "ok" | "error"; text: string } | null>(null);
const editable = computed(() => !!props.filePath && !!props.editor);

const changes = (): SourceEdit[] => {
  const out: SourceEdit[] = [];
  if (element.value.text && text.value !== element.value.text) out.push({ kind: "text", before: element.value.text, after: text.value });
  for (const { property } of STYLES) {
    const value = (styles.value[property] ?? "").trim();
    if (value && value !== element.value.style?.[property]) out.push({ kind: "style", property, value });
  }
  return out;
};

const save = async () => {
  const edits = changes();
  if (!props.editor || !edits.length) {
    note.value = { kind: "error", text: t("workbench.edit_nothing") };
    return;
  }
  busy.value = true;
  note.value = null;
  const out = await props.editor(props.filePath, { selector: element.value.selector, tagName: element.value.tagName }, edits);
  busy.value = false;
  if (out.saved) {
    element.value = { ...element.value, text: element.value.text ? text.value : element.value.text, style: { ...element.value.style, ...Object.fromEntries(edits.flatMap((e) => (e.kind === "style" ? [[e.property, e.value]] : []))) } };
    note.value = { kind: "ok", text: t("workbench.edit_saved", { path: out.path }) };
    props.onSaved();
  } else {
    note.value = { kind: "error", text: FAILED[out.code]() };
  }
};

const onText = (e: Event) => (text.value = (e.target as HTMLInputElement).value);
const onStyleInput = (property: StyleProperty, e: Event) => (styles.value = { ...styles.value, [property]: (e.target as HTMLInputElement).value });
</script>

<template>
  <form class="belement" :aria-label="t('workbench.edit_picked_element')" data-action-submit="browser.edit-save" @submit.prevent="save">
    <header class="belement-head">
      <code :title="element.selector">{{ element.selector }}</code>
      <button type="button" data-action="browser.edit-close" @click="onClose">{{ t("workbench.edit_close") }}</button>
    </header>
    <p v-if="!editable" class="studio-browser-note">{{ t("workbench.edit_not_file") }}</p>
    <label v-if="element.text !== undefined && element.text !== ''" class="belement-field">
      <span>{{ t("workbench.edit_text") }}</span>
      <input :value="text" :disabled="!editable || busy" :spellcheck="false" @input="onText" />
    </label>
    <label v-for="{ property, label } in STYLES" :key="property" class="belement-field">
      <span>{{ label() }}</span>
      <input :value="styles[property] ?? ''" :disabled="!editable || busy" :spellcheck="false" @input="onStyleInput(property, $event)" />
    </label>
    <div class="belement-foot">
      <button type="submit" data-action="browser.edit-save" :disabled="!editable || busy">{{ busy ? t("workbench.edit_saving") : t("workbench.edit_save") }}</button>
      <p v-if="note" :role="note.kind === 'error' ? 'alert' : 'status'" :class="note.kind === 'error' ? 'studio-browser-error' : 'studio-browser-note'">{{ note.text }}</p>
    </div>
  </form>
</template>