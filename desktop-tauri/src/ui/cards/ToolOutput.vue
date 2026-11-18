<script lang="ts">
// Every shape here was read off real tool results, not inferred: a search hit
// is "path:line:text", a context line repeats the path with a dash, a summary
// is "path: N matches", ls is "name<TAB>bytes". Anything that does not parse
// cleanly falls back to the terminal block rather than being forced into a
// list — a wrong list reads as authoritative, a raw block only reads as raw.

import { computed, defineComponent, h, ref, unref, watch, type PropType, type VNode } from "vue";
import { t } from "../../i18n";
import { useStartsOpen } from "../../state/foldpref";
import { bytes } from "../../i18n/format";
import type { Bound } from "../../port/wire";
import { splitPath } from "../args";

const MATCH = /^(.+?):(\d+):([\s\S]*)$/;
// ls prints a directory as "name/" with no size and a file as "name<TAB>bytes".
const LS_DIR = /^(.+\/)$/;
const LS_FILE = /^(.+)\t(\d+)$/;

// Below this share of lines fitting the shape, the output is something else.
const CONFIDENT = 0.8;

export interface Hit {
  path: string;
  line: string;
  text: string;
  more: string[];
}

export function parseHits(out: string): { hits: Hit[]; note: string } | null {
  const all = out.split("\n").filter((l) => l.trim());
  // grep appends its own truncation and timeout notes. They are the tool
  // telling you the result is partial, not stray output — pull them out before
  // judging the shape, and keep them to show.
  const note = all.filter((l) => TRAILER.test(l)).join(" ");
  const lines = all.filter((l) => !TRAILER.test(l));
  if (lines.length === 0) return null;
  const hits: Hit[] = [];
  let loose = 0;
  for (const line of lines) {
    const m = MATCH.exec(line);
    if (m) {
      hits.push({ path: m[1], line: m[2], text: m[3].trim(), more: [] });
      continue;
    }
    const cur = hits[hits.length - 1];
    // A context line repeats the hit's own path, so strip it by literal rather
    // than by a pattern that a path containing "-12-" would fool.
    if (cur && line.startsWith(cur.path)) {
      cur.more.push(line.slice(cur.path.length).replace(/^[-:]\d+[-:]/, "").trim());
      continue;
    }
    loose++;
  }
  return hits.length && loose / lines.length <= 1 - CONFIDENT ? { hits, note } : null;
}

export interface Row {
  name: string;
  n: string;
}

export function parseRows(out: string, re: RegExp): Row[] | null {
  const lines = out.split("\n").filter((l) => l.trim());
  if (lines.length === 0) return null;
  const rows = lines.map((l) => re.exec(l)).filter((m): m is RegExpExecArray => m !== null);
  if (rows.length / lines.length < CONFIDENT) return null;
  // A shape with no count still makes a manifest; the column just stays empty.
  return rows.map((m) => ({ name: m[1], n: m[2] ?? "" }));
}

export function parseListing(out: string): Row[] | null {
  const lines = out.split("\n").filter((l) => l.trim());
  if (lines.length === 0) return null;
  const rows: Row[] = [];
  for (const l of lines) {
    const f = LS_FILE.exec(l);
    if (f) {
      rows.push({ name: f[1], n: f[2] });
      continue;
    }
    const d = LS_DIR.exec(l);
    if (d) rows.push({ name: d[1], n: "" });
  }
  return rows.length / lines.length < CONFIDENT ? null : rows;
}

// A manifest is read-only where the spec draws it. Only a merged read run makes
// its rows pickable, because collapsing the cards would otherwise put the file
// contents out of reach entirely.
export const Peek = defineComponent({
  name: "Peek",
  props: {
    rows: { type: Array as PropType<{ name: string; n: string }[]>, required: true },
    unit: { type: String, required: true },
    onPick: { type: Function as PropType<(i: number) => void>, default: undefined },
    at: { type: Number, default: undefined },
  },
  setup(props) {
    return () => h("div", { class: "peek" }, props.rows.map((r, i) => {
      const [dir, name] = splitPath(r.name);
      const body = [
        h("span", null, [dir ? h("span", { class: "d" }, dir) : null, name]),
        r.n ? h("span", { class: "n" }, [r.n, props.unit ? ` ${props.unit}` : null]) : null,
      ];
      return props.onPick
        ? h("button", { class: "row", key: i, "data-on": props.at === i ? "" : undefined, onClick: () => props.onPick!(i) }, body)
        : h("div", { class: "row", key: i }, body);
    }));
  },
});

// One result row: what it is, where it came from, what it said. A grep hit and a
// web search result are the same shape — a claim with a source — so the spec
// draws them with one set of elements rather than two lookalike lists.
export interface Row3 {
  t: string;
  u: string;
  q: string[];
}

export const hitRows = (hits: Hit[]): Row3[] =>
  hits.map((h) => {
    const [dir, name] = splitPath(h.path);
    return { t: name, u: `${dir}${name}:${h.line}`, q: [h.text, ...h.more] };
  });

export const Hits = defineComponent({
  name: "Hits",
  props: {
    rows: { type: Array as PropType<Row3[]>, required: true },
    note: { type: String, default: undefined },
  },
  setup(props) {
    return () => h("div", { class: "hits" }, [
      ...props.rows.map((r, i) => h("div", { class: "hit-row", key: i }, [
        h("div", { class: "i" }, String(i + 1)),
        h("div", null, [
          h("div", { class: "t" }, r.t),
          r.u ? h("div", { class: "u", title: r.u }, r.u) : null,
          r.q.some(Boolean) ? h("div", { class: "q" }, [r.q[0], ...r.q.slice(1).map((m, k) => h("div", { key: k }, m))]) : null,
        ]),
      ])),
      props.note ? h("div", { class: "hit-row" }, [h("div", { class: "i" }), h("div", { class: "u" }, props.note)]) : null,
    ]);
  },
});

// A progress meter redraws by returning to column zero, and a browser draws a
// carriage return as a space, so each line is resolved the way a terminal
// would leave it: every frame overwrites the start of the one before.
function overstrike(text: string): string {
  if (!text.includes("\r")) return text;
  return text
    .split("\n")
    .map((line) => {
      const frames = (line.endsWith("\r") ? line.slice(0, -1) : line).split("\r");
      return frames.reduce((shown, frame) => frame + shown.slice(frame.length));
    })
    .join("\n");
}

export const Term = defineComponent({
  name: "Term",
  props: {
    text: { type: String, required: true },
    one: { type: Boolean, default: false },
  },
  setup(props) {
    const text = computed(() => overstrike(props.text));
    return () => h("pre", { class: "term", "data-one": props.one ? "" : undefined }, text.value);
  },
});

// A provider-run search returns the listing the kernel formats for the model:
// "- **title**" and, indented under it, "<url>". A result can arrive with no URL
// at all, so the title alone still counts as a row.
const SEARCH_TITLE = /^-\s+\*\*(.+?)\*\*\s*$/;
const SEARCH_URL = /^\s+<(\S+)>\s*$/;

export function parseSearchResults(out: string): Row3[] | null {
  const lines = out.split("\n").filter((l) => l.trim());
  if (lines.length === 0) return null;
  const rows: Row3[] = [];
  let loose = 0;
  for (const line of lines) {
    const title = SEARCH_TITLE.exec(line);
    if (title) {
      rows.push({ t: title[1], u: "", q: [] });
      continue;
    }
    const url = SEARCH_URL.exec(line);
    const cur = rows[rows.length - 1];
    if (url && cur && !cur.u) {
      cur.u = url[1];
      continue;
    }
    loose++;
  }
  return rows.length && loose / lines.length <= 1 - CONFIDENT ? rows : null;
}

// glob returns one path per line. Requiring a separator keeps a one-line error
// message from being dressed up as a manifest of one file.
export const PATH = /^\s*([^\s]*[/\\][^\s]*)\s*$/;
const TRAILER = /^\.\.\. \(/;

// A directory listing and a glob result are both manifests, which is what .peek
// is for. grep prints "path:line:text" and gets the excerpt list. Everything
// else — a shell run, an MCP result, a listing that did not parse — is a block.
function shapeFor(name: string, text: string): VNode {
  if (name === "ls") {
    const rows = parseListing(text);
    if (rows) return h(Peek, { rows, unit: "B" });
  }
  if (name === "glob") {
    const rows = parseRows(text, PATH);
    if (rows) return h(Peek, { rows, unit: "" });
  }
  if (name === "grep") {
    const found = parseHits(text);
    if (found) return h(Hits, { rows: hitRows(found.hits), note: found.note });
  }
  if (name === "web_search") {
    const rows = parseSearchResults(text);
    if (rows) return h(Hits, { rows });
  }
  // A surface exists to cut a block of machine text into the page. One line is
  // not a block: it costs the same 42px of padded ground as forty do, and a
  // settled call whose whole output is "ok" then weighs what a test run does.
  return text.includes("\n") ? h(Term, { text }) : h(Term, { text, one: true });
}

// One visual budget for every shape above, carried by CSS rather than counted
// here. Clipping keeps the content scannable and complete.
const opened = new Map<string, boolean>();

interface ClipProps {
  id?: string;
  deps?: unknown;
}

const Clip = defineComponent((props: ClipProps, { slots }) => {
  const start = useStartsOpen("output");
  const touched = ref<boolean | undefined>(props.id ? opened.get(props.id) : undefined);
  const open = computed(() => touched.value ?? unref(start));
  const over = ref(false);
  const body = ref<HTMLDivElement | null>(null);
  // Measuring reads scrollHeight, which forces layout — so it keys off the
  // content and the clip state, not every render the turn above it causes.
  watch([() => props.deps, open], (_v, _o, onCleanup) => {
    const el = body.value;
    if (!el) return;
    const measure = () => (over.value = el.scrollHeight > el.clientHeight + 1);
    // Off the commit, onto the next frame. Opening a long transcript mounts
    // hundreds of these at once, and reading scrollHeight inside the commit
    // makes each card force a layout of the whole document.
    const raf = requestAnimationFrame(() => {
      // Collapsing now takes time, and mid-transition this measures an
      // intermediate height. While one is running, wait for it to settle.
      if (!el.getAnimations?.().length) measure();
    });
    el.addEventListener("transitionend", measure);
    onCleanup(() => {
      cancelAnimationFrame(raf);
      el.removeEventListener("transitionend", measure);
    });
  }, { immediate: true, flush: "post" });
  const toggle = () => {
    const next = !open.value;
    touched.value = next;
    // Keyed by call, so reopening a card the user already expanded does not ask
    // them to expand it again.
    if (props.id) opened.set(props.id, next);
  };
  return () => h("div", { class: "out-clip", "data-open": open.value ? "" : undefined, "data-over": over.value ? "" : undefined }, [
    h("div", { class: "out-body", ref: body }, slots.default?.()),
    (over.value || open.value) ? h("button", { class: "out-more", onClick: toggle }, open.value ? t("common.collapse") : t("chat.show_all")) : null,
  ]);
}, { name: "Clip", props: ["id", "deps"] });

// The three outcomes are not the same news, and a card that renders them alike
// is why "folded" and "the model never saw this" looked identical. Only
// truncation is a warning; the other two say where the rest is.
export const BoundNote = defineComponent({
  name: "BoundNote",
  props: { bound: { type: Object as PropType<Bound>, default: undefined } },
  setup(props) {
    return () => {
      const bound = props.bound;
      if (!bound) return null;
      if (bound.kind === "spilled") {
        return h("div", { class: "bound", title: bound.path }, t("chat.full_output_saved_to_disk", { lines: bound.lines ?? 0, size: bytes(bound.bytes ?? 0) }));
      }
      if (bound.kind === "windowed") return h("div", { class: "bound" }, t("chat.showing_the_start_the_model"));
      return h("div", { class: "bound bad" }, t("chat.the_model_received_only_kept", { kept: bytes(bound.keptBytes ?? 0), size: bytes(bound.bytes ?? 0) }));
    };
  },
});

export const ToolOutput = defineComponent({
  name: "ToolOutput",
  props: {
    name: { type: String, required: true },
    text: { type: String, required: true },
    bound: { type: Object as PropType<Bound>, default: undefined },
    id: { type: String as PropType<string | undefined>, default: undefined },
  },
  setup(props) {
    return () => [
      h(Clip, { id: props.id, deps: props.text }, { default: () => shapeFor(props.name, props.text) }),
      h(BoundNote, { bound: props.bound }),
    ];
  },
});

export default ToolOutput;
</script>