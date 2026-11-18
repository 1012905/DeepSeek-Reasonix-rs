<script lang="ts">
import { computed, defineComponent, h, type PropType } from "vue";
import EffortSlider from "./EffortSlider.vue";
import { MenuRow, type MenuItem } from "./Menu.vue";
import { useFolded } from "./usefold";

const isLevel = (it: MenuItem) => !it.header && it.toggle === undefined && it.strength !== undefined;

export default defineComponent({
  name: "EffortColumn",
  props: {
    items: { type: Array as PropType<MenuItem[]>, required: true },
    current: { type: String, required: true },
    onPick: { type: Function as PropType<(value: string) => void>, required: true },
  },
  setup(props) {
    const phone = useFolded("phone");
    const levels = computed(() => props.items.filter(isLevel));
    const heading = computed(() => props.items.find((it) => it.header));
    // A phone has no room for a row of words per level: they become one slider.
    const slide = computed(() => phone.value && levels.value.length >= 2);
    return () => props.items.map((it) => {
      if (it.header) {
        return h("div", { key: it.value, class: "mi head" }, [
          h("span", { class: "lb" }, it.label),
          it.right && h("span", { class: "rt" }, it.right),
        ]);
      }
      if (slide.value && isLevel(it)) {
        return it === levels.value[0]
          ? h(EffortSlider, { key: "slider", name: heading.value?.label ?? "", model: heading.value?.right, levels: levels.value, current: props.current, onPick: props.onPick })
          : null;
      }
      return h(MenuRow, { key: it.value, it, id: { "data-action": "reasoning.effort" }, current: props.current, onTake: props.onPick });
    });
  },
});
</script>
