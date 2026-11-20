<script lang="ts">
import { defineComponent, h, type PropType } from "vue";
import { t } from "../../i18n";
import { useWatched } from "../Cost.vue";

const AFTER_MS = 10_000;

/** One call, stopped alone. It appears only once a call has run long enough to
 *  look stuck, so a quick call never offers it. */
export const StopCall = defineComponent({
  name: "StopCall",
  props: { onStop: { type: Function as PropType<() => void>, required: true } },
  setup(props) {
    const watched = useWatched(true);
    return () => watched.value < AFTER_MS
      ? null
      : h("button", { type: "button", class: "stop-call", "data-action": "tool.stop", onClick: props.onStop }, t("chat.stop_only_this_call"));
  },
});

/** The line under a running step in the activity group, on the same clock as
 *  StopCall: what it has produced so far, and a way to stop it alone. */
export const StepWait = defineComponent({
  name: "StepWait",
  props: {
    onStop: { type: Function as PropType<(() => void) | undefined>, default: undefined },
    quiet: { type: Boolean, required: true },
  },
  setup(props) {
    const watched = useWatched(true);
    return () => {
      if (watched.value < AFTER_MS || (props.quiet && !props.onStop)) return null;
      return h("div", { class: "step-wait" }, [
        !props.quiet ? t("panels.no_output_yet") : null,
        !props.quiet && props.onStop ? " · " : null,
        props.onStop ? h("button", { type: "button", class: "step-stop", "data-action": "tool.stop", onClick: props.onStop }, t("chat.stop_this_one")) : null,
      ]);
    };
  },
});

export default StopCall;
</script>
