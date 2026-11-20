<script lang="ts">
import { computed, defineComponent, h, onMounted, ref, type PropType } from "vue";
import { LOOK_FAMILY, SETTINGS, type Section } from "./prefsnav";
import { hiddenAnchors } from "./settingPages";

/** Keeps the sheet to what one page is.
 *
 *  Only two cases need a rule, and both are the page disagreeing with a sibling
 *  of the same section: the look pages share one family and each shows only its
 *  own, and a page that is one of several on a section keeps the others' rows
 *  out of sight. A block the catalogue files under a *different* section is
 *  deliberately left alone — a page may draw a folded page's blocks on purpose
 *  (`model.vue` draws `providers`, `session.vue` draws `hooks`), and hiding them
 *  would blank exactly the pages that do.
 *
 *  Rendered into .prefs-col so the rules go away with the column. The column is
 *  rebuilt per section, so its mount is also where the panel goes back to the
 *  top: arriving at a page starts at its title, while arriving at a block is
 *  scrolled to by useSettingLanding after this has run. */
export default defineComponent({
  name: "SettingsPageStyle",
  props: {
    section: { type: String as PropType<Section>, required: true },
    page: { type: String as PropType<string | undefined>, default: undefined },
  },
  setup(props) {
    const style = ref<HTMLStyleElement | null>(null);

    const hidden = computed(() => {
      if (LOOK_FAMILY.includes(props.section)) {
        return SETTINGS
          .filter((entry) => LOOK_FAMILY.includes(entry.section) && entry.section !== props.section)
          .map((entry) => entry.anchor);
      }
      return props.page ? hiddenAnchors(props.section, props.page) : [];
    });

    const css = computed(() => {
      if (hidden.value.length === 0) return "";
      const blocks = hidden.value.map((id) => `.prefs-col #set-${id},.prefs-col [data-setting="${id}"]`).join(",");
      const wrappers = hidden.value.map((id) => `.prefs-col details:has(> [data-setting="${id}"])`).join(",");
      return `${blocks}{display:none}\n${wrappers}{display:none}`;
    });

    onMounted(() => {
      const panel = style.value?.closest<HTMLElement>(".prefs-main");
      if (panel) panel.scrollTop = 0;
    });

    return () => (css.value ? h("style", { ref: style, "data-settings-page-style": props.section }, css.value) : null);
  },
});
</script>
