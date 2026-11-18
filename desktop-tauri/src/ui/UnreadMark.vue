<script lang="ts">
import { defineComponent, h } from "vue";
import { plural, t } from "../i18n";

export const UnreadDot = defineComponent({
  name: "UnreadDot",
  setup() {
    return () => h("i", { class: "unread-dot", role: "img", "aria-label": t("common.unread"), title: t("common.unread") });
  },
});

/** A row's lead dot. Unread is drawn by the dot alone, so the dot carries the word. */
export const LeadDot = defineComponent({
  name: "LeadDot",
  props: { unread: { type: Boolean, default: undefined } },
  setup(props) {
    return () => {
      const label = props.unread ? t("common.unread") : undefined;
      return h("span", { class: "ds-lead" }, [
        h("i", { class: "pip ds-dot", role: label ? "img" : undefined, "aria-label": label, title: label }),
      ]);
    };
  },
});

export const UnreadCount = defineComponent({
  name: "UnreadCount",
  props: { n: { type: Number, required: true } },
  setup(props) {
    return () => {
      if (props.n <= 0) return null;
      const label = plural(props.n, "session.n_1_unread_session", "session.n_unread_sessions");
      return h("i", { class: "unread-count", role: "img", "aria-label": label, title: label }, props.n > 9 ? "9+" : String(props.n));
    };
  },
});

export default UnreadDot;
</script>