<script lang="ts">
import { defineComponent, type PropType, type VNodeChild } from "vue";
import type { StudioIconName } from "../StudioIcon.vue";
import { Sym } from "../Sym.vue";

const Content = defineComponent({
  props: { value: { type: null as unknown as PropType<VNodeChild>, default: undefined } },
  setup(props) {
    return () => (props.value === undefined || props.value === null || props.value === false ? null : props.value);
  },
});

export default defineComponent({
  name: "NoteCard",
  components: { Content, Sym },
  props: {
    icon: { type: String as PropType<StudioIconName>, required: true },
    level: { type: String as PropType<"warn" | "err">, default: undefined },
    gone: { type: Boolean, default: false },
    lead: { type: null as unknown as PropType<VNodeChild>, required: true },
    strong: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    meta: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    tail: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    action: { type: null as unknown as PropType<VNodeChild>, default: undefined },
  },
});
</script>

<template>
  <div class="note" :data-lvl="level" :data-lead="strong ? '' : undefined" :data-gone="gone ? '' : undefined">
    <div class="note-hd">
      <Sym :icon="icon" />
      <span class="note-nm"><Content :value="lead" /></span>
      <b v-if="strong" class="note-strong"><Content :value="strong" /></b>
      <span v-if="meta" class="note-meta"><Content :value="meta" /></span>
      <Content :value="tail" />
      <slot name="action"><Content :value="action" /></slot>
    </div>
    <slot />
  </div>
</template>
