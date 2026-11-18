<script setup lang="ts">
import { computed, defineComponent, h, ref, type PropType } from "vue";
import { useStartsOpen } from "../../state/foldpref";
import type { Tool } from "../../port/wire";
import { argOf, splitPath } from "../args";
import { Sym, iconFor } from "../Sym.vue";
import Cost from "../Cost.vue";
import { Hits, PATH, Peek, Term, hitRows, parseHits, parseListing, parseRows } from "./ToolOutput.vue";
import { toolErrorDetail, toolFailed, toolOutputEchoed, toolRefusalReason } from "./outcome";
import { t as tr } from "../../i18n";

// read_file numbers every line it returns, so the count is in the output rather
// than guessed from it.
const NUMBERED = /^\s*\d+→/;
const countLines = (out?: string) => (out ? out.split("\n").filter((l) => NUMBERED.test(l)).length : 0);
const countRows = (out?: string) => (out ? out.split("\n").filter((l) => l.trim()).length : 0);
const countHits = (out?: string) => (out ? (parseHits(out)?.hits.length ?? countRows(out)) : 0);

interface Row {
  id: string;
  name: string;
  dir: string;
  n: string;
  failed: boolean;
}

// What each call contributes to the manifest: the left column is what it
// touched, the right is the operation and its count.
function rowOf(t: Tool): Row {
  const arg = argOf(t.args, "path", "file_path", "pattern", "query") || t.args || "—";
  const state = { id: t.id ?? "", failed: toolFailed(t) };
  switch (t.name) {
    case "read_file": {
      const [dir, name] = splitPath(arg);
      return { ...state, dir, name, n: state.failed ? tr("common.failed") : tr("chat.read_n_lines", { n: countLines(t.output) }) };
    }
    case "grep": {
      const where = argOf(t.args, "path", "file_path");
      return { ...state, dir: "", name: `${tr("搜索")} “${arg}”${where ? ` ${where}` : ""}`, n: state.failed ? tr("common.failed") : tr("chat.n_matches", { n: countHits(t.output) }) };
    }
    case "glob":
      return { ...state, dir: "", name: `${tr("查找文件")} ${arg}`, n: state.failed ? tr("common.failed") : tr("common.n", { n: countRows(t.output) }) };
    default:
      return { ...state, dir: "", name: `${tr("列出目录")} ${arg}`, n: state.failed ? tr("common.failed") : tr("settings.n_items", { n: countRows(t.output) }) };
  }
}

// The fold summary is what the run was, by tool: a manifest of thirty rows says
// nothing at a glance that "read_file ×4 · grep 命中 14 处 · glob ×3" does not.
function summarise(tools: Tool[]): string {
  const reads = tools.filter((t) => t.name === "read_file").length;
  const greps = tools.filter((t) => t.name === "grep");
  const globs = tools.filter((t) => t.name === "glob").length;
  const lists = tools.filter((t) => t.name === "ls").length;
  const hits = greps.reduce((n, t) => n + countHits(t.output), 0);
  return [
    reads && tr("chat.read_files_n", { n: reads }),
    greps.length && tr("chat.search_n_matches", { n: hits }),
    globs && tr("chat.find_files_n", { n: globs }),
    lists && tr("chat.list_directories_n", { n: lists }),
  ]
    .filter(Boolean)
    .join(" · ");
}

// A picked row opens the call's own result in the shape that result has.
const Body = defineComponent({
  name: "ReadsBody",
  props: { tool: { type: Object as PropType<Tool>, required: true } },
  setup(props) {
    return () => {
      const tool = props.tool;
      const out = toolOutputEchoed(tool) ? undefined : tool.output;
      const refusal = toolRefusalReason(tool);
      const errorDetail = toolErrorDetail(tool);
      let body = null;
      if (out) {
        if (tool.name === "grep") {
          const found = parseHits(out);
          if (found) body = h(Hits, { rows: hitRows(found.hits), note: found.note });
        }
        if (!body && tool.name === "glob") {
          const rows = parseRows(out, PATH);
          if (rows) body = h(Peek, { rows, unit: "" });
        }
        if (!body && tool.name === "ls") {
          const rows = parseListing(out);
          if (rows) body = h(Peek, { rows, unit: "B" });
        }
        if (!body) body = h(Term, { text: out });
      }
      return [
        body,
        refusal ? h("div", { class: "txt", "data-refusal": tool.refusalCode }, refusal) : null,
        errorDetail ? h("div", { class: "txt bad" }, errorDetail) : null,
        tool.bound?.kind === "truncated" ? h("div", { class: "bound bad" }, tr("chat.the_output_is_incomplete_some")) : null,
      ];
    };
  },
});

// A run of lookups is one step, not one card each. The manifest answers what
// was touched; the result itself is one click away.
const props = defineProps<{ tools: Tool[] }>();
const open = ref(-1);
const failed = computed(() => props.tools.filter(toolFailed).length);
const start = useStartsOpen("steps", false, () => failed.value > 0);
const touched = ref<boolean | null>(null);
const expanded = computed(() => touched.value ?? start.value);
const rows = computed(() => props.tools.map(rowOf));
const files = computed(() => props.tools.filter((t) => t.name === "read_file").length);
const searches = computed(() => props.tools.filter((t) => t.name === "grep" || t.name === "glob").length);
const headline = computed(() => (files.value === props.tools.length
  ? tr("chat.read_n_files", { n: files.value })
  : searches.value === props.tools.length
    ? tr("chat.completed_n_searches", { n: searches.value })
    : tr("chat.inspected_n_items", { n: props.tools.length })));
const picked = computed(() => (open.value >= 0 ? props.tools[open.value] : undefined));

const onToggle = (event: Event) => {
  const el = event.currentTarget as HTMLDetailsElement;
  if (el.open !== expanded.value) touched.value = el.open;
};
</script>

<template>
  <div class="call" :data-state="failed ? 'failed' : 'done'">
    <div class="g">
      <Sym :icon="iconFor('read_file')" />
    </div>
    <div class="c">
      <details class="tool-disclosure reads-disclosure" :open="expanded" @toggle="onToggle">
        <summary class="hl">
          <span class="nm" :title="headline">{{ headline }}</span>
          <span class="arg" :title="summarise(tools)">{{ summarise(tools) }}</span>
          <span v-if="failed > 0" class="fail">{{ tr("chat.n_failed", { n: failed }) }}</span>
          <Cost :tools="tools" />
          <span
            class="tool-state"
            :data-state="failed ? 'failed' : 'done'"
            :aria-label="tr(failed ? 'common.failed' : 'common.done_2')"
            :title="tr(failed ? 'common.failed' : 'common.done_2')"
          />
          <span class="tool-fold" aria-hidden="true" />
        </summary>
        <div class="out">
          <div class="peek">
            <button
              v-for="(r, i) in rows"
              :key="r.id || `${r.name}#${i}`"
              class="row"
              :data-call="r.id || undefined"
              :data-bad="r.failed ? '' : undefined"
              :data-on="open === i ? '' : undefined"
              @click="open = open === i ? -1 : i"
            >
              <span><span v-if="r.dir" class="d">{{ r.dir }}</span>{{ r.name }}</span>
              <span class="n">{{ r.n }}</span>
            </button>
          </div>
          <Body v-if="picked" :tool="picked" />
        </div>
      </details>
    </div>
  </div>
</template>
