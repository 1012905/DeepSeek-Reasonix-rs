<script lang="ts">
import { defineComponent, h, type PropType, type VNode } from "vue";
import type { ExtensionViewNode } from "../../port/wire";
import { t } from "../../i18n";
import SurfaceKit from "../surface/SurfaceKit.vue";
import SurfaceDiffPreview from "../surface/SurfaceDiffPreview.vue";

// A composed surface is a tree of primitives the host draws with its own
// components. Input kinds (`field`, `choice_card`, `editable_text`, `map`,
// `comparison`) belong to a submitted form, not to a display view, so they draw
// nothing here — the same rule the React catalog applies outside inputs mode.
function meter(node: ExtensionViewNode, key: string): VNode {
  const pct = Math.round(Math.max(0, Math.min(1, node.progress ?? 0)) * 100);
  return h("div", { key, class: "xv-meter", role: "progressbar", "aria-valuenow": pct }, [
    h("i", { style: { width: `${pct}%` } }),
    node.label ? h("span", { class: "lb" }, node.label) : null,
  ]);
}

function dashboard(node: ExtensionViewNode, key: string): VNode | null {
  const dataset = node.dataset;
  if (!dataset) return null;
  const cell = (v: number | undefined) => (v ?? 0).toLocaleString(undefined, { maximumFractionDigits: 2 });
  return h("div", { key, class: "xv-stack" }, [
    node.label ? h("p", { class: "xv-t", "data-tone": "strong" }, node.label) : null,
    h("div", { class: "rich-table" }, h("table", { "aria-label": node.label ?? t("chat.data_dashboard") }, [
      h("thead", null, h("tr", null, [
        h("th", null, t("chat.classifying")),
        ...dataset.series.map((s) => h("th", { key: s.id }, s.unit ? `${s.label} (${s.unit})` : s.label)),
      ])),
      h("tbody", null, dataset.categories.map((category, i) => h("tr", { key: category }, [
        h("th", null, category),
        ...dataset.series.map((s) => h("td", { key: s.id }, cell(s.values[i]))),
      ]))),
    ])),
  ]);
}

function renderNode(node: ExtensionViewNode, onAction: ((id: string) => void) | undefined, key: string): VNode | null {
  switch (node.kind) {
    case "text":
      return h("span", { key, class: "xv-t", "data-tone": node.tone }, node.value ?? "");
    case "markdown":
      return h("p", { key, class: "xv-md" }, node.value ?? "");
    case "row":
    case "stack":
      return h("div", { key, class: node.kind === "row" ? "xv-row" : "xv-stack" },
        (node.children ?? []).map((child, i) => renderNode(child, onAction, `${key}.${i}`)));
    case "kv":
      return h("div", { key, class: "xv-kv" }, [
        h("span", { class: "k" }, node.key ?? ""),
        h("span", { class: "v" }, node.value ?? ""),
      ]);
    case "meter":
      return meter(node, key);
    case "pip":
      return h("i", { key, class: "pip", "data-tone": node.tone });
    case "button":
      return h("button", {
        key,
        class: "act",
        "data-action": "extensions.invoke",
        disabled: !node.actionId,
        onClick: () => onAction?.(node.actionId ?? ""),
      }, node.label ?? "");
    case "divider":
      return h("hr", { key, class: "xv-hr" });
    case "diff_preview":
      return node.diff ? h(SurfaceDiffPreview, { key, diff: node.diff, label: node.label }) : null;
    case "dashboard":
      return dashboard(node, key);
    case "field":
    case "choice_card":
    case "editable_text":
    case "map":
    case "comparison":
      return null;
  }
}

export default defineComponent({
  name: "ExtensionView",
  props: {
    body: { type: Array as PropType<ExtensionViewNode[]>, required: true },
    onAction: { type: Function as PropType<(id: string) => void>, default: undefined },
  },
  setup(props) {
    return () => h(SurfaceKit, null, {
      default: () => h("div", { class: "xview" }, [
        h("div", { class: "xv-stack" }, props.body.map((node, i) => renderNode(node, props.onAction, `n${i}`))),
      ]),
    });
  },
});
</script>
