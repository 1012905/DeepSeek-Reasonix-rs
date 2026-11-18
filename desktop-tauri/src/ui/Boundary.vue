<script lang="ts">
import { defineComponent, onErrorCaptured, ref, watch, type VNodeChild } from "vue";

// Model output is arbitrary text and the renderers that read it can throw —
// KaTeX does it for one undefined control sequence. A throw during render
// unmounts the whole tree, so the window goes blank over a single bad token.
// Falling one message back to its source text keeps the rest readable.
//
// retryKey is what the failure was about. When it changes the children get
// another try: a throw on half a streamed answer, or a renderer chunk that
// failed to load once, must not hold the finished answer in the fallback.
interface BoundaryProps {
  fallback?: VNodeChild;
  retryKey?: unknown;
}

export default defineComponent((props: BoundaryProps, { slots }) => {
  const failed = ref(false);
  const key = ref<unknown>(props.retryKey);

  watch(() => props.retryKey, (k) => {
    if (k !== key.value) { failed.value = false; key.value = k; }
  });

  onErrorCaptured((error) => {
    failed.value = true;
    console.warn("[reasonix] a block fell back to its source text:", error);
    return false;
  });

  return () => (failed.value ? (props.fallback ?? slots.fallback?.() ?? null) : (slots.default?.() ?? null));
}, { name: "Boundary", props: ["fallback", "retryKey"] });
</script>
