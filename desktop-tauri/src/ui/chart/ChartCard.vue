<script setup lang="ts">
import { computed, defineComponent, h, onBeforeUnmount, onMounted, ref, useId, type Component } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { plotOf } from "./model";
import { clip } from "./format";
import { BarPlot, LinePlot, PiePlot, type Hot } from "./plots";
import ChartTable from "./ChartTable.vue";
import type { ChartMark, ChartSpec } from "./spec";

const props = defineProps<{ spec: ChartSpec; callId?: string }>();

const uid = useId();
const hatch = `${uid}-hatch`;
const read = ref<string | null>(null);
const hot: Hot = (text) => { read.value = text; };

const marks = computed(() => props.spec.marks);
const describe = computed(() => {
  const kinds = marks.value.map((m) => m.type === "bar" ? t("chat.bar_chart") : m.type === "line" ? t("chat.line_chart") : t("chat.pie_chart"));
  return t("chat.kind_title_n_rows", { kind: [...new Set(kinds)].join(" · "), title: props.spec.title, n: props.spec.data.rows.length });
});
const axes = computed(() => {
  const one = (a?: { title?: string; unit?: string }) => [a?.title, a?.unit && `(${a.unit})`].filter(Boolean).join(" ");
  return [one(props.spec.x_axis), one(props.spec.y_axis)].filter(Boolean).join(" × ");
});
const markComponent = (m: ChartMark): Component => m.type === "bar" ? BarPlot : m.type === "line" ? LinePlot : PiePlot;
const seriesNames = (m: ChartMark): string[] => m.type === "pie" ? [] : plotOf(props.spec, m).series.map((s) => s.label);
const scrolls = (m: ChartMark) => m.type !== "pie";

// A mark wider than its pane scrolls sideways; the affordance tracks the scroll
// extent so the edge fades only while there is more to reach.
const Scroller = defineComponent({
  name: "ChartScroller",
  props: { scrolls: { type: Boolean, required: true }, label: { type: String, required: true } },
  setup(p, { slots }) {
    const box = ref<HTMLDivElement | null>(null);
    const more = ref({ left: false, right: false });
    let watch: ResizeObserver | undefined;
    const measure = () => {
      const el = box.value;
      if (!el) return;
      const next = { left: el.scrollLeft > 1, right: el.scrollLeft + el.clientWidth < el.scrollWidth - 1 };
      if (more.value.left !== next.left || more.value.right !== next.right) more.value = next;
    };
    onMounted(() => {
      measure();
      if (p.scrolls && box.value) { watch = new ResizeObserver(measure); watch.observe(box.value); }
    });
    onBeforeUnmount(() => watch?.disconnect());
    return () => h("div", { class: "chart-scroll", "data-more-left": more.value.left || undefined, "data-more-right": more.value.right || undefined }, [
      h("div", {
        class: "chart-pane", ref: box, onScroll: measure,
        "data-action": p.scrolls ? "chart.scroll" : undefined,
        tabindex: p.scrolls ? 0 : undefined,
        role: p.scrolls ? "region" : undefined,
        "aria-label": p.scrolls ? p.label : undefined,
      }, slots.default?.()),
    ]);
  },
});
</script>

<template>
  <div class="call" data-k="chart" :data-call="callId || undefined">
    <div class="g"><StudioIcon name="chart" /></div>
    <div class="c">
      <div class="hl">
        <span class="nm">{{ t("chat.chart") }}</span>
        <span class="arg" :title="spec.title">{{ spec.title }}</span>
      </div>
      <figure class="chart" role="group" :aria-label="describe">
        <svg width="0" height="0" aria-hidden="true" class="chart-defs">
          <defs>
            <pattern :id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1="0" y1="0" x2="0" y2="6" class="hatch-line" />
            </pattern>
          </defs>
        </svg>
        <div v-for="(m, i) in marks" :key="i" class="chart-mark">
          <Scroller :scrolls="scrolls(m)" :label="t('chat.chart_title_scrolls_sideways', { title: spec.title })">
            <component :is="markComponent(m)" :spec="spec" :mark="m" :hot="hot" :hatch="hatch" />
          </Scroller>
          <ul v-if="seriesNames(m).length >= 2" class="chart-key">
            <li v-for="(n, k) in seriesNames(m)" :key="n">
              <i :class="`cs${k % 5} sv${Math.min(Math.floor(k / 5), 2)}`" />
              <span class="nm">{{ clip(n, 28) }}</span>
            </li>
          </ul>
        </div>
        <figcaption v-if="axes" class="chart-axes">{{ axes }}</figcaption>
        <div class="chart-read" aria-live="polite">{{ read ?? t("chat.hover_or_focus_the_chart") }}</div>
      </figure>
      <ChartTable :spec="spec" :id="`${uid}-data`" />
    </div>
  </div>
</template>