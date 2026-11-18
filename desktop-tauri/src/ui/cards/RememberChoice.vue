<script lang="ts">
import { defineComponent, h, type PropType } from "vue";
import { t } from "../../i18n";

export type Remembered = "session" | "always";

const LABEL: Record<Remembered, string> = {
  session: "chat.allow_this_kind_for_this",
  always: "chat.always_allow_this_kind_stop",
};

/** The quieter half of a prompt's footer: the choices that answer this question
 *  and every later one of its kind. Which of them is drawn is the kernel's call
 *  (`options` names what it will honour here), and the line under them says where
 *  each one is kept, so remembering is never a surprise. */
export const RememberChoice = defineComponent({
  name: "RememberChoice",
  props: {
    options: { type: Array as PropType<readonly Remembered[]>, required: true },
    target: { type: String, required: true },
    busy: { type: String, required: true },
    disabled: { type: Boolean, required: true },
    onChoose: { type: Function as PropType<(choice: Remembered) => void>, required: true },
  },
  setup(props) {
    return () => {
      if (props.options.length === 0) return null;
      return props.options.map((choice) => h("button", {
        key: choice,
        class: "btn",
        "data-weak": "",
        "data-action": "decision.tool",
        "data-target": props.target,
        "data-value": choice,
        "aria-describedby": `remember-where-${props.target}`,
        disabled: props.disabled,
        onClick: () => props.onChoose(choice),
      }, props.busy === choice ? t("common.submitting") : t(LABEL[choice])));
    };
  },
});

const SESSION_KEPT = "chat.allow_for_this_session_lasts";

// Where an always answer is kept, for the questions that do not write a
// permission rule. Keyed on the tool id the kernel puts on the card.
const ALWAYS_KEPT_AT: Record<string, string> = {
  paid_search: "chat.always_allow_adds_this_paid",
  network_egress: "chat.always_allow_adds_this_domain",
  mcp_signin: "chat.always_allow_records_this_sign",
};

const ALWAYS_KEPT_AS_RULE = "chat.always_allow_is_saved_as";

/** Where the remembered choices live, as one sentence for the footer's note.
 *  "Always" is a permission rule for an ordinary tool and something else for the
 *  questions in ALWAYS_KEPT_AT, so the sentence follows the tool. */
export const RememberWhere = defineComponent({
  name: "RememberWhere",
  props: {
    options: { type: Array as PropType<readonly Remembered[]>, required: true },
    target: { type: String, required: true },
    tool: { type: String, default: undefined },
  },
  setup(props) {
    return () => {
      if (props.options.length === 0) return null;
      const always = (props.tool && ALWAYS_KEPT_AT[props.tool]) || ALWAYS_KEPT_AS_RULE;
      const text = [props.options.includes("session") && SESSION_KEPT, props.options.includes("always") && always].filter((x): x is string => !!x).map((x) => t(x)).join(" ");
      return h("div", { class: "apv-note", id: `remember-where-${props.target}` }, text);
    };
  },
});

export default RememberChoice;
</script>