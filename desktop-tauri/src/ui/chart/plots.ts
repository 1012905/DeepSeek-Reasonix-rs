import { defineComponent, h, type PropType, type VNode } from "vue";
import type { ChartMark, ChartSpec } from "./spec";
import { plotOf, slicesOf, stackOf, yScaleOf, type Plot, type YScale } from "./model";
import { clip, formatValue } from "./format";
import { t } from "../../i18n";
import { count, pct } from "../../i18n/format";

export const W = 640;
export const H = 280;
const TOP = 14;
const BOTTOM = 30;
const RIGHT = 14;
const FOCUSABLE_MAX = 60;
const LABEL_MAX = 12;
const GLYPH = 6.4;

export type Hot = (text: string | null) => void;

interface Frame {
  spec: ChartSpec;
  mark: ChartMark;
  hot: Hot;
  hatch: string;
}

const frameProps = {
  spec: { type: Object as PropType<ChartSpec>, required: true },
  mark: { type: Object as PropType<ChartMark>, required: true },
  hot: { type: Function as PropType<Hot>, required: true },
  hatch: { type: String, required: true },
} as const;

const seriesClass = (i: number) => `cs${i % 5} sv${Math.min(Math.floor(i / 5), 2)}`;

function pointProps(label: string, hot: Hot, focusable: boolean) {
  return {
    tabindex: focusable ? 0 : undefined,
    "aria-label": label,
    role: "img",
    onMouseenter: () => hot(label),
    onMouseleave: () => hot(null),
    onFocus: () => hot(label),
    onBlur: () => hot(null),
  };
}

// A value drawn beside its mark only while the plot is sparse and the text fits the room its mark has.
const fits = (text: string, room: number) => text.length * GLYPH <= room + 8;

function ValueLabel(x: number, y: number, text: string): VNode {
  return h("text", { class: "dl", x, y, "text-anchor": "middle", "aria-hidden": "true" }, text);
}

function Axes(y: YScale, left: number, spec: ChartSpec): VNode {
  const base = H - BOTTOM;
  const at = (v: number) => base - y.at(v) * (base - TOP);
  return h("g", { class: "ax", "aria-hidden": "true" }, [
    ...y.ticks.map((v) => h("g", { key: v }, [
      h("line", { class: "grid", x1: left, x2: W - RIGHT, y1: at(v), y2: at(v) }),
      h("text", { class: "tick", x: left - 6, y: at(v), "text-anchor": "end", "dominant-baseline": "middle" }, formatValue(v, { ...spec.y_axis, unit: undefined })),
    ])),
    h("line", { class: "base", x1: left, x2: W - RIGHT, y1: base, y2: base }),
  ]);
}

function leftMargin(y: YScale, spec: ChartSpec): number {
  const widest = Math.max(...y.ticks.map((v) => formatValue(v, { ...spec.y_axis, unit: undefined }).length));
  return Math.min(120, 14 + widest * 6.6);
}

function XLabels(cats: string[], xs: number[], left: number): VNode {
  const room = (W - RIGHT - left) / Math.max(1, cats.length);
  const widest = Math.min(14, Math.max(...cats.map((c) => [...c].length)));
  const every = Math.max(1, Math.ceil((widest * 6.4 + 10) / room));
  return h("g", { class: "xl", "aria-hidden": "true" }, cats.map((c, i) => (i % every === 0
    ? h("text", { key: i, class: "tick", x: xs[i], y: H - BOTTOM + 16, "text-anchor": "middle" }, clip(c, 14))
    : null)));
}

export const BarPlot = defineComponent({
  name: "BarPlot",
  props: frameProps,
  setup(props) {
    return () => {
      const { spec, mark, hot, hatch } = props as Frame;
      const p = plotOf(spec, mark);
      const stack = mark.stacked ? stackOf(p.series) : null;
      const all = stack ? [...stack.lo.flat(), ...stack.hi.flat()] : p.series.flatMap((s) => s.values.filter((v): v is number => v !== null));
      const y = yScaleOf(all, spec.y_axis, true);
      const left = leftMargin(y, spec);
      const base = H - BOTTOM;
      const band = (W - RIGHT - left) / Math.max(1, p.categories.length);
      const slot = stack ? band * 0.7 : (band * 0.76) / Math.max(1, p.series.length);
      const at = (v: number) => base - y.at(v) * (base - TOP);
      const zero = y.log ? base : at(0);
      const focusable = p.categories.length * p.series.length <= FOCUSABLE_MAX;
      const labelled = p.categories.length * p.series.length <= LABEL_MAX;
      const xs = p.categories.map((_, i) => left + band * (i + 0.5));
      return h("svg", { viewBox: `0 0 ${W} ${H}`, class: "chart-svg", style: { minWidth: `${W}px` } }, [
        Axes(y, left, spec),
        ...p.series.map((s, k) => s.values.map((v, i) => {
          if (v === null) return null;
          const a = stack ? at(stack.hi[k][i]) : at(v);
          const b = stack ? at(stack.lo[k][i]) : zero;
          const x = stack ? xs[i] - slot / 2 : left + band * i + band * 0.12 + slot * k;
          const label = `${s.label} · ${p.categories[i]} · ${formatValue(v, spec.y_axis)}`;
          return h("g", { key: `${k}:${i}`, class: `bar ${seriesClass(k)}`, ...pointProps(label, hot, focusable) }, [
            h("title", null, label),
            h("rect", { x, y: Math.min(a, b), width: Math.max(1, slot - 1), height: Math.max(1, Math.abs(b - a)) }),
            k >= 5 ? h("rect", { class: "hatch", x, y: Math.min(a, b), width: Math.max(1, slot - 1), height: Math.max(1, Math.abs(b - a)), fill: `url(#${hatch})` }) : null,
            labelled && !stack && fits(formatValue(v, spec.y_axis), slot) ? ValueLabel(x + slot / 2, Math.min(a, b) - 4, formatValue(v, spec.y_axis)) : null,
          ]);
        })),
        XLabels(p.categories, xs, left),
      ]);
    };
  },
});

function xPositions(p: Plot, left: number): number[] {
  const right = W - RIGHT;
  if (!p.positions) return p.categories.map((_, i) => (p.categories.length === 1 ? (left + right) / 2 : left + 12 + ((right - left - 24) * i) / (p.categories.length - 1)));
  const lo = Math.min(...p.positions);
  const hi = Math.max(...p.positions);
  return p.positions.map((v) => (hi === lo ? (left + right) / 2 : left + 12 + ((v - lo) / (hi - lo)) * (right - left - 24)));
}

export const LinePlot = defineComponent({
  name: "LinePlot",
  props: frameProps,
  setup(props) {
    return () => {
      const { spec, mark, hot } = props as Frame;
      const p = plotOf(spec, mark);
      const y = yScaleOf(p.series.flatMap((s) => s.values.filter((v): v is number => v !== null)), spec.y_axis, false);
      const left = leftMargin(y, spec);
      const base = H - BOTTOM;
      const at = (v: number) => base - y.at(v) * (base - TOP);
      const xs = xPositions(p, left);
      const order = p.positions ? p.categories.map((_, i) => i).sort((a, b) => p.positions![a] - p.positions![b]) : p.categories.map((_, i) => i);
      const dots = p.categories.length * p.series.length <= FOCUSABLE_MAX;
      const labelled = p.categories.length * p.series.length <= LABEL_MAX;
      const gap = Math.min(Infinity, ...order.slice(1).map((i, j) => xs[i] - xs[order[j]]));
      return h("svg", { viewBox: `0 0 ${W} ${H}`, class: "chart-svg", style: { minWidth: `${W}px` } }, [
        Axes(y, left, spec),
        ...p.series.map((s, k) => {
          const runs: number[][] = [];
          let run: number[] = [];
          for (const i of order) {
            const v = s.values[i];
            if (v === null) { if (run.length) runs.push(run); run = []; continue; }
            run.push(i);
          }
          if (run.length) runs.push(run);
          return h("g", { key: k, class: `line ${seriesClass(k)}` }, [
            ...runs.map((indices, r) => {
              const d = indices.map((i, j) => `${j ? "L" : "M"}${xs[i].toFixed(1)} ${at(s.values[i]!).toFixed(1)}`).join("");
              const hit = indices.length === 1 ? `${d}L${xs[indices[0]].toFixed(1)} ${at(s.values[indices[0]]!).toFixed(1)}` : d;
              return h("g", { key: r }, [
                h("path", { d }),
                h("path", {
                  class: "line-hit",
                  d: hit,
                  "aria-hidden": "true",
                  onMousemove: (e: MouseEvent) => {
                    const rect = (e.currentTarget as SVGPathElement).ownerSVGElement!.getBoundingClientRect();
                    if (!rect.width) return;
                    const x = ((e.clientX - rect.left) / rect.width) * W;
                    const i = indices.reduce((best, next) => Math.abs(xs[next] - x) < Math.abs(xs[best] - x) ? next : best);
                    hot(`${s.label} · ${p.categories[i]} · ${formatValue(s.values[i]!, spec.y_axis)}`);
                  },
                  onMouseleave: () => hot(null),
                }, [h("title", null, s.label)]),
              ]);
            }),
            ...s.values.map((v, i) => {
              if (v === null) return null;
              const label = `${s.label} · ${p.categories[i]} · ${formatValue(v, spec.y_axis)}`;
              return dots
                ? h("g", { key: i }, [
                  h("circle", { class: "pt", cx: xs[i], cy: at(v), r: 3.4, ...pointProps(label, hot, true) }, [h("title", null, label)]),
                  labelled && fits(formatValue(v, spec.y_axis), gap) ? ValueLabel(xs[i], at(v) - 9, formatValue(v, spec.y_axis)) : null,
                ])
                : null;
            }),
          ]);
        }),
        XLabels(order.map((i) => p.categories[i]), order.map((i) => xs[i]), left),
      ]);
    };
  },
});

const TAU = Math.PI * 2;

function arc(cx: number, cy: number, r: number, inner: number, a0: number, a1: number): string {
  const pt = (rad: number, a: number) => `${(cx + rad * Math.sin(a)).toFixed(2)} ${(cy - rad * Math.cos(a)).toFixed(2)}`;
  if (a1 - a0 >= TAU - 1e-6) {
    const ring = (rad: number) => `M${cx} ${cy - rad}A${rad} ${rad} 0 1 1 ${cx} ${cy + rad}A${rad} ${rad} 0 1 1 ${cx} ${cy - rad}`;
    return inner ? `${ring(r)}${ring(inner)}` : ring(r);
  }
  const big = a1 - a0 > Math.PI ? 1 : 0;
  const outer = `M${pt(r, a0)}A${r} ${r} 0 ${big} 1 ${pt(r, a1)}`;
  return inner ? `${outer}L${pt(inner, a1)}A${inner} ${inner} 0 ${big} 0 ${pt(inner, a0)}Z` : `${outer}L${cx} ${cy}Z`;
}

export const PiePlot = defineComponent({
  name: "PiePlot",
  props: frameProps,
  setup(props) {
    return () => {
      const { spec, mark, hot } = props as Frame;
      const slices = slicesOf(spec, mark);
      const total = slices.reduce((n, s) => n + s.value, 0);
      if (slices.length === 0) return h("div", { class: "chart-empty" }, t("chat.nothing_to_draw"));
      const r = 104;
      let angle = 0;
      return h("div", { class: "chart-pie" }, [
        h("svg", { viewBox: `0 0 ${W / 2} ${H}`, class: "chart-svg pie" }, slices.map((s, i) => {
          const a0 = angle;
          angle += (s.value / total) * TAU;
          const label = `${s.label} · ${formatValue(s.value, spec.y_axis)} · ${pct(s.value / total, 1)}`;
          return h("path", { key: i, class: `slice ${seriesClass(i)}`, d: arc(W / 4, H / 2, r, mark.donut ? r * 0.58 : 0, a0, angle), "fill-rule": "evenodd", ...pointProps(label, hot, slices.length <= FOCUSABLE_MAX) }, [h("title", null, label)]);
        })),
        h("ul", { class: "chart-key" }, [
          ...slices.slice(0, 12).map((s, i) => h("li", { key: i }, [
            h("i", { class: seriesClass(i) }),
            h("span", { class: "nm" }, clip(s.label, 24)),
            h("span", { class: "v" }, pct(s.value / total, 1)),
          ])),
          slices.length > 12 ? h("li", { class: "more" }, `+${count(slices.length - 12)}`) : null,
        ]),
      ]);
    };
  },
});
