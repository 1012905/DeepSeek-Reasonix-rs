<script setup lang="ts">
import { computed, useId, type CSSProperties } from "vue";
import type { ExtensionViewNode, SurfaceOption } from "../../port/wire";

const props = defineProps<{
  node: ExtensionViewNode;
  value: unknown;
  disabled: boolean;
  change: (value: unknown) => void;
}>();

const f = computed(() => props.node.field!);
const group = useId();
const textValue = computed(() => (props.value === undefined ? "" : String(props.value)));
const inputType = computed(() => (f.value.type === "number" ? "number" : "text"));
const onText = (e: Event) => props.change((e.target as HTMLInputElement | HTMLTextAreaElement).value);
const onBool = (e: Event) => props.change((e.target as HTMLInputElement).checked);
const isOn = (o: SurfaceOption) => (f.value.type === "choice" ? props.value === o.value : Array.isArray(props.value) && props.value.includes(o.value));
const take = (o: SurfaceOption) => {
  if (f.value.type === "choice") props.change(isOn(o) ? undefined : o.value);
  else {
    const chosen = Array.isArray(props.value) ? props.value : [];
    props.change(isOn(o) ? chosen.filter(v => v !== o.value) : [...chosen, o.value]);
  }
};
// Mantine's Radio.Card walked same-name radios in document order on the arrow
// keys; the card here is that button, so it keeps the walk.
const onCardKeydown = (e: KeyboardEvent) => {
  if (!["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(e.code)) return;
  e.preventDefault();
  const siblings = Array.from(document.querySelectorAll(`[role="radio"][name="${group}"]`));
  const currentIndex = siblings.findIndex((element) => element === e.target);
  const nextIndex = currentIndex + 1 >= siblings.length ? 0 : currentIndex + 1;
  const prevIndex = currentIndex - 1 < 0 ? siblings.length - 1 : currentIndex - 1;
  const to = e.code === "ArrowDown" || e.code === "ArrowRight" ? nextIndex : prevIndex;
  const next = siblings[to] as HTMLElement | undefined;
  next?.focus();
  next?.click();
};

const gridStyle = computed<CSSProperties>(() => ({
  display: "grid",
  gap: "12px",
  gridTemplateColumns: props.node.kind === "choice_card" ? "repeat(auto-fill, minmax(220px, 1fr))" : "1fr",
}));
const labelTextStyle: CSSProperties = { fontSize: "14px", fontWeight: 500 };
const dimStyle: CSSProperties = { fontSize: "14px", color: "var(--muted)" };
const previewStyle: CSSProperties = { display: "block", border: "1px solid var(--border)", padding: "12px" };
const previewTextStyle: CSSProperties = { whiteSpace: "pre-wrap", overflowWrap: "anywhere", fontSize: "14px" };
const cardStyle: CSSProperties = { textAlign: "start", padding: "16px", border: "1px solid var(--border)", borderRadius: "8px", background: "var(--surface)", cursor: "pointer" };
</script>

<template>
  <textarea
    v-if="node.kind === 'editable_text'"
    class="mantine-Textarea-input"
    data-action="surface.edit"
    :data-target="node.key"
    :aria-label="node.label"
    :aria-required="f.required || undefined"
    :required="f.required"
    :readonly="disabled"
    rows="6"
    :value="textValue"
    @input="onText"
  />
  <label v-else-if="f.type === 'bool'" class="mantine-Checkbox-root" data-action="surface.edit" :data-target="node.key">
    <input type="checkbox" class="mantine-Checkbox-input" :checked="value === true" :disabled="disabled" @change="onBool" />
    <span>{{ node.label }}</span>
  </label>
  <input
    v-else-if="f.type !== 'choice' && f.type !== 'multichoice'"
    class="mantine-TextInput-input"
    data-action="surface.edit"
    :data-target="node.key"
    :aria-label="node.label"
    :aria-required="f.required || undefined"
    :required="f.required"
    :readonly="disabled"
    :type="inputType"
    :min="f.min"
    :max="f.max"
    step="any"
    :value="textValue"
    @input="onText"
  />
  <div v-else class="xv-stack">
    <p :style="labelTextStyle">{{ node.label }}{{ f.required ? " *" : "" }}</p>
    <div :role="f.type === 'choice' ? 'radiogroup' : 'group'" :aria-label="node.label" :aria-required="f.required || undefined" :style="gridStyle">
      <template v-for="o in f.options" :key="o.value">
        <button
          v-if="f.type === 'choice'"
          type="button"
          class="mantine-RadioCard-root"
          role="radio"
          :aria-checked="isOn(o)"
          :name="group"
          :disabled="disabled"
          :data-checked="isOn(o) || undefined"
          data-action="surface.edit"
          :data-target="node.key"
          :style="cardStyle"
          @click="take(o)"
          @keydown="onCardKeydown"
        >
          <span class="xv-row" style="align-items: flex-start; flex-wrap: nowrap">
            <span class="mantine-RadioIndicator-root" aria-hidden="true" />
            <span class="xv-stack" style="min-width: 0">
              <span :style="labelTextStyle">{{ o.label }}</span>
              <span v-if="o.description" :style="dimStyle">{{ o.description }}</span>
              <span v-if="o.preview" :style="previewStyle"><span :style="previewTextStyle">{{ o.preview }}</span></span>
            </span>
          </span>
        </button>
        <button
          v-else
          type="button"
          class="mantine-CheckboxCard-root"
          role="checkbox"
          :aria-checked="isOn(o)"
          :disabled="disabled"
          :data-checked="isOn(o) || undefined"
          data-action="surface.edit"
          :data-target="node.key"
          :style="cardStyle"
          @click="take(o)"
        >
          <span class="xv-row" style="align-items: flex-start; flex-wrap: nowrap">
            <span class="mantine-CheckboxIndicator-root" aria-hidden="true" />
            <span class="xv-stack" style="min-width: 0">
              <span :style="labelTextStyle">{{ o.label }}</span>
              <span v-if="o.description" :style="dimStyle">{{ o.description }}</span>
              <span v-if="o.preview" :style="previewStyle"><span :style="previewTextStyle">{{ o.preview }}</span></span>
            </span>
          </span>
        </button>
      </template>
    </div>
  </div>
</template>