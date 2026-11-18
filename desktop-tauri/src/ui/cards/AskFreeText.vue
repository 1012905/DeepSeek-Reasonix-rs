<script lang="ts">
import { defineComponent, h, reactive, ref, watch, type PropType } from "vue";
import type { AgentPort } from "../../port/port";
import { usePromptRefine } from "../PromptRefine";

interface ImeHandlers {
  onCompositionStart: () => void;
  onCompositionEnd: () => void;
}

const NO_REFINE = async (draft: string) => draft;

/** The free-text answer of one question. A rewrite is offered the way the
 *  composer offers it, by the same hook; an adopted rewrite is joined onto one
 *  line because the box is one line. */
export default defineComponent({
  name: "AskFreeText",
  props: {
    value: { type: String, required: true },
    label: { type: String, required: true },
    placeholder: { type: String, required: true },
    readOnly: { type: Boolean, required: true },
    inputRef: { type: Function as PropType<(el: HTMLInputElement | null) => void>, required: true },
    onChange: { type: Function as PropType<(text: string) => void>, required: true },
    onFocus: { type: Function as PropType<(() => void) | undefined>, default: undefined },
    onEnter: { type: Function as PropType<() => void>, required: true },
    isIme: { type: Function as PropType<(e: { isComposing?: boolean; keyCode?: number }) => boolean>, required: true },
    imeHandlers: { type: Object as PropType<ImeHandlers>, required: true },
    refine: { type: Function as PropType<AgentPort["refinePrompt"]>, default: undefined },
  },
  setup(props) {
    const refine = reactive(usePromptRefine(
      () => ({ refinePrompt: props.refine ?? NO_REFINE }),
      () => props.value,
      (next: string) => props.onChange(next.replace(/\s*\n+\s*/g, " ").trim()),
    ));
    const box = ref<HTMLInputElement | null>(null);
    watch(box, (el) => props.inputRef(el), { immediate: true });
    const onKeyDown = (e: KeyboardEvent) => {
      if (props.isIme(e)) return;
      if (props.refine && !props.readOnly && refine.onKey(e)) return;
      if (e.key !== "Enter" || props.readOnly) return;
      e.preventDefault();
      props.onEnter();
    };
    return () => [
      h("div", { class: "other-field" }, [
        h("input", {
          ref: box,
          "data-action-keydown": "ask.answer",
          "aria-label": props.label,
          value: props.value,
          readonly: props.readOnly,
          placeholder: props.placeholder,
          onInput: (e: Event) => props.onChange((e.target as HTMLInputElement).value),
          onFocus: () => props.onFocus?.(),
          onKeydown: onKeyDown,
          onCompositionStart: props.imeHandlers.onCompositionStart,
          onCompositionEnd: props.imeHandlers.onCompositionEnd,
        }),
        props.refine && !props.readOnly ? refine.button : null,
      ]),
      props.refine && !props.readOnly ? refine.card : null,
    ];
  },
});
</script>
