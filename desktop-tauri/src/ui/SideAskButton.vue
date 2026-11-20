<script lang="ts">
import { defineComponent, h, onMounted, ref, Teleport, useId, watch, type PropType, type VNodeChild } from "vue";
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";

/** The side question's switch, drawn beside attach. Whether the panel is open is
 *  the pane's to say; this reports the press, draws the panel it was handed into
 *  the composer it sits in, and takes the focus back when that panel closes. */
export default defineComponent({
  name: "SideAskButton",
  props: {
    open: { type: Boolean, required: true },
    onToggle: { type: Function as PropType<() => void>, required: true },
    panel: { type: null as unknown as PropType<VNodeChild> },
  },
  setup(props) {
    const tip = useId();
    const button = ref<HTMLButtonElement | null>(null);
    let was = props.open;
    const host = ref<Element | null>(null);
    onMounted(() => { host.value = button.value?.closest(".compose") ?? null; });
    watch(() => props.open, (open) => {
      if (was && !open && (!document.activeElement || document.activeElement === document.body)) button.value?.focus();
      was = open;
    });
    return () => [
      h("button", {
        ref: button,
        type: "button",
        class: "mode plain attach studio-attach",
        "data-action": "side.toggle",
        "aria-label": t("composer.side_question"),
        "aria-describedby": tip,
        "aria-pressed": props.open,
        onClick: props.onToggle,
      }, [
        h(StudioIcon, { name: "chat" }),
        h("span", { class: "studio-control-tip", id: tip, role: "tooltip" }, [
          h("b", {}, t("composer.side_question")),
          h("span", {}, t("composer.read_only_not_in_the")),
        ]),
      ]),
      props.open && host.value && props.panel ? h(Teleport, { to: host.value }, [props.panel]) : null,
    ];
  },
});
</script>
