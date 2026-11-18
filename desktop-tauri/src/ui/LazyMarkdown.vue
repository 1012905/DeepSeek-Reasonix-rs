<script lang="ts">
import { defineAsyncComponent, defineComponent, h, shallowRef, Suspense, type Component, type PropType } from "vue";
import type { LocalRefs } from "./Markdown.vue";

// A failed import stays failed for the life of the module graph, so one chunk
// that did not load (a dropped connection, a dev server mid-reload, an update
// that replaced the files) would leave every reply as source text. A failure
// swaps in a fresh async component, and the next render loads again.
const Markdown = shallowRef<Component>(loader());

function loader(): Component {
  return defineAsyncComponent(async () => {
    try {
      const module = await import("./Markdown.vue");
      return module.default;
    } catch (err) {
      Markdown.value = loader();
      throw err;
    }
  });
}

export const LazyMarkdown = defineComponent({
  name: "LazyMarkdown",
  props: {
    text: { type: String, required: true },
    streaming: { type: Boolean, default: false },
    local: { type: Object as PropType<LocalRefs>, default: undefined },
    images: { type: Boolean, default: undefined },
  },
  setup(props) {
    return () => h(Suspense, null, {
      default: () => h(Markdown.value, { text: props.text, streaming: props.streaming, local: props.local, images: props.images }),
      fallback: () => h("div", { class: "md", style: { whiteSpace: "pre-wrap" } }, props.text),
    });
  },
});

export default LazyMarkdown;
</script>
