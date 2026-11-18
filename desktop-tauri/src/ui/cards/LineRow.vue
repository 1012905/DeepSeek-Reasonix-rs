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

/** The inline step of the transcript grammar: a bare status icon, the title, the
 *  mono subject and the meta on one row over a hairline, with an optional detail
 *  line under it. */
export default defineComponent({
  name: "LineRow",
  components: { Content, Sym },
  props: {
    icon: { type: String as PropType<StudioIconName>, required: true },
    title: { type: null as unknown as PropType<VNodeChild>, required: true },
    subject: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    subjectTitle: { type: String, default: undefined },
    meta: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    action: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    detail: { type: null as unknown as PropType<VNodeChild>, default: undefined },
    level: { type: String as PropType<"ok" | "warn" | "err">, default: undefined },
    gone: { type: Boolean, default: false },
  },
});
</script>

<template>
  <div class="line-row" :data-lvl="level" :data-gone="gone ? '' : undefined">
    <div class="lr-main">
      <Sym :icon="icon" />
      <span class="lr-nm"><Content :value="title" /></span>
      <span v-if="subject" class="lr-arg" :title="subjectTitle"><Content :value="subject" /></span>
      <span v-if="meta" class="lr-meta"><Content :value="meta" /></span>
      <slot name="action"><Content :value="action" /></slot>
    </div>
    <div v-if="detail" class="lr-detail"><Content :value="detail" /></div>
  </div>
</template>
