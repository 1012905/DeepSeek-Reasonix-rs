<script lang="ts">
import { defineComponent, h } from "vue";
import { t } from "../i18n";
import type { FeedbackTab } from "./Feedback.vue";

/** Every entry to feedback lands on the list when something wants attention. */
export const feedbackEntryTab = (unread: number): FeedbackTab => (unread > 0 ? "mine" : "send");

export const FeedbackBadge = defineComponent({
  name: "FeedbackBadge",
  props: { unread: { type: Number, required: true } },
  setup(props) {
    return () => {
      if (props.unread <= 0) return null;
      return [
        h("i", { class: "fbk-badge", "aria-hidden": "true", title: t("common.n_to_review", { n: props.unread }) }, props.unread > 9 ? "9+" : String(props.unread)),
        h("span", { class: "sr-only" }, t("common.n_to_review", { n: props.unread })),
      ];
    };
  },
});

export default FeedbackBadge;
</script>