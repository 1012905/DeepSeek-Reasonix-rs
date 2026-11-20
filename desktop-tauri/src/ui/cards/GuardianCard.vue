<script lang="ts">
import { defineComponent, h, type PropType } from "vue";
import { t } from "../../i18n";
import type { Guardian } from "../../port/wire";
import LineRow from "./LineRow.vue";
import NoteCard from "./NoteCard.vue";

export default defineComponent({
  name: "GuardianCard",
  props: { g: { type: Object as PropType<Guardian>, required: true } },
  setup(props) {
    return () => {
      const g = props.g;
      const risk = g.risk_level ?? "unknown";
      const riskLabel = risk === "low" ? t("chat.low_risk") : risk === "medium" ? t("chat.medium_risk") : risk === "high" ? t("common.high_risk") : risk === "critical" ? t("chat.critical_risk") : t("chat.risk_unknown");
      if (g.outcome === "allow" && (risk === "low" || risk === "unknown")) {
        return h(LineRow, {
          icon: "shieldCheck", level: "ok",
          title: `${t("chat.guardian_review")} · ${t("chat.allowed")}`,
          subject: g.subject, subjectTitle: g.subject, meta: riskLabel,
          detail: g.rationale || undefined,
        });
      }
      const bars = risk === "low" ? 1 : risk === "medium" ? 2 : risk === "unknown" ? 0 : 3;
      const verdict = g.outcome === "deny" ? t("chat.blocked") : g.outcome === "allow" ? t("chat.allowed") : g.outcome;
      return h(NoteCard, {
        icon: "shield",
        level: g.outcome === "deny" ? "err" : "warn",
        lead: [t("chat.guardian_review"), " · ", h("span", { class: "note-verdict" }, verdict)],
        tail: [
          h("span", { class: "note-risk" }, riskLabel),
          h("span", { class: "note-gauge", "aria-hidden": "true" }, [1, 2, 3].map((n) => h("i", { key: n, "data-on": n <= bars ? "" : undefined }))),
        ],
      }, {
        default: () => [
          h("div", { class: "note-subject", title: g.subject }, g.subject),
          g.rationale ? h("div", { class: "note-text" }, g.rationale) : null,
        ],
      });
    };
  },
});
</script>
