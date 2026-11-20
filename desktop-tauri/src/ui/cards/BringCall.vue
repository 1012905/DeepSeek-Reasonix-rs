<script lang="ts">
import { defineComponent, h, type PropType } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

/** Shows this call's own result again at the end of the transcript, so a reader
 *  does not have to find the card the model summarised. */
export const BringCall = defineComponent({
  name: "BringCall",
  props: { onBring: { type: Function as PropType<() => void>, required: true } },
  setup(props) {
    return () => h("button", {
      type: "button",
      class: "bring-call",
      "data-action": "tool.bring",
      "aria-label": t("chat.show_this_result_again_at"),
      title: t("chat.show_this_result_again_at"),
      onClick: (event: MouseEvent) => {
        // It sits in the call's heading, which is also what folds the card.
        event.preventDefault();
        event.stopPropagation();
        props.onBring();
      },
    }, h(StudioIcon, { name: "arrow" }));
  },
});

export default BringCall;
</script>
