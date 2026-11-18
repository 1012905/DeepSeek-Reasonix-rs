<script lang="ts">
import { defineComponent, type PropType } from "vue";
import { SegChoice } from "../../SettingRow.vue";

/** SegChoice is a render helper with no prop declaration of its own, so a
 *  template binding cannot reach its callback: Vue only camelises a hyphenated
 *  key that matches a declared prop. This wrapper declares one, under a name
 *  the compiler leaves alone, and calls the helper. */
export default defineComponent({
  name: "LookSeg",
  props: {
    label: { type: String, required: true },
    value: { type: String, required: true },
    options: { type: Array as PropType<readonly (readonly [string, string])[]>, required: true },
    action: { type: String, required: true },
    pick: { type: Function as PropType<(next: string) => void>, required: true },
    disabled: { type: Boolean, default: false },
  },
  setup(props) {
    return () =>
      SegChoice({
        label: props.label,
        value: props.value,
        options: props.options,
        action: props.action,
        disabled: props.disabled,
        onPick: props.pick,
      });
  },
});
</script>
