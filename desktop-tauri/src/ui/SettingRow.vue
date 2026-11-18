<script lang="ts">
import { h, type Slots, type VNodeChild } from "vue";
import { ApplyNote, ScopeTag } from "./Group.vue";

export const SegChoice = <V extends string>(props: {
  label: string;
  value: V;
  options: readonly (readonly [V, string])[];
  disabled?: boolean;
  action: string;
  onPick: (next: V) => void;
}): VNodeChild =>
  h("div", { class: "seg", "data-text": "true", role: "radiogroup", "aria-label": props.label },
    props.options.map(([id, name]) =>
      h("button", {
        key: id,
        role: "radio",
        "data-action": props.action,
        "data-value": id,
        "aria-checked": props.value === id,
        disabled: props.disabled,
        onClick: () => props.onPick(id),
      }, name)));

export const PillButton = (
  props: { action: string; target?: string; disabled?: boolean; onClick: () => void },
  { slots }: { slots: Slots },
): VNodeChild =>
  h("button", {
    type: "button",
    class: "spill",
    "data-action": props.action,
    "data-target": props.target,
    disabled: props.disabled,
    onClick: props.onClick,
  }, slots.default?.());
</script>

<script setup lang="ts">
defineProps<{ id: string; title: string; desc?: string; more?: string }>();
</script>

<template>
  <div class="srow" :id="`set-${id}`" :data-setting="id">
    <div class="srow-meta">
      <span class="srow-name">{{ title }}<ScopeTag :id="id" /></span>
      <span v-if="desc" class="srow-desc" :title="more ?? desc">{{ desc }}</span>
      <ApplyNote :id="id" />
    </div>
    <div v-if="$slots.default" class="srow-ctl"><slot /></div>
  </div>
</template>