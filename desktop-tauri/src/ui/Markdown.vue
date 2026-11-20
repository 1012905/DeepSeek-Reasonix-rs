<script lang="ts">
import { computed, defineComponent, h, isVNode, shallowRef, toValue, watch, type FunctionalComponent, type MaybeRefOrGetter, type PropType, type VNode, type VNodeChild } from "vue";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import { toHast } from "mdast-util-to-hast";
import { t } from "../i18n";
import type { FileRef } from "../port/workspace";
import CodeBlock from "./CodeBlock.vue";
import { remarkCodeSpanLink, remarkTrimAutolink } from "./autolink";
import { OUTSIDE_IMAGE, remarkFileLinks, remarkLocalImages } from "./filelinks";
import { useRevealed } from "./reveal";

// Model output is untrusted markup, so raw HTML is parsed and then cut back to
// an allowlist. These four tags carry meaning no markdown syntax expresses;
// nothing that can navigate, load, or script survives the filter. Subscript and
// superscript ride here rather than on a shorthand plugin: Pandoc's ~2~ collides
// with GFM's ~~strikethrough~~, and one mechanism beats two that fight.
const schema = {
  ...defaultSchema,
  tagNames: [...(defaultSchema.tagNames ?? []), "mark", "sub", "sup", "kbd"],
  attributes: {
    ...defaultSchema.attributes,
    // KaTeX marks its output with classes; sanitizing runs before it, so the
    // math wrappers remark-math emits have to survive to be rendered.
    "*": [...(defaultSchema.attributes?.["*"] ?? []), "className"],
  },
};

// KaTeX and its fonts are about half the bundle, and most coding sessions never
// produce a formula, so it is fetched the first time one appears. No lookbehind
// — older WebKit treats it as a syntax error and takes the whole page down.
const MATH = /\$\$[\s\S]+?\$\$|\$[^\n$]+\$/;
const EMOJI = /:[a-zA-Z0-9_+-]+:/;
// Only a fence that named its language. rehype-highlight's own detection stays
// off: guessing a grammar paints prose as code.
const FENCED = /^```[a-zA-Z]/m;

// Models reach for LaTeX's own delimiters as readily as for dollars, and
// remark-math reads only dollars. Rewriting them keeps one parser instead of
// two. Code is split out first: a \[ inside a fence is text, not an equation.
const CODE = /(```[\s\S]*?```|`[^`\n]*`)/g;

function normalizeMath(md: string) {
  return md
    .split(CODE)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part
            .replace(/\\\[([\s\S]+?)\\\]/g, (_, m) => `\n\n$$${m}$$\n\n`)
            .replace(/\\\(([\s\S]+?)\\\)/g, (_, m) => `$${m}$`),
    )
    .join("");
}

type Plugin = unknown;
type Slot = { plugin: Plugin | null; loading: Promise<void> | null; load: () => Promise<Plugin> };

const KATEX: Slot = {
  plugin: null,
  loading: null,
  // Red source text beats an exception: the message stays readable and the
  // render cannot take the window down with it.
  load: () =>
    Promise.all([import("rehype-katex"), import("katex/dist/katex.min.css")]).then(
      ([mod]) => [mod.default, { throwOnError: false }] as Plugin,
    ),
};
/** Classes, not inline colours: a highlighter writing its own hex values would
 *  leave the token system and stop following the theme. */
const HLJS: Slot = { plugin: null, loading: null, load: () => import("rehype-highlight").then((m) => m.default) };
const EMOJIS: Slot = { plugin: null, loading: null, load: () => import("remark-gemoji").then((m) => m.default) };

/** Fetched the first time the text calls for one. Every read of a slot goes
 *  through a thunk because a plugin is a function: a ref would otherwise hold
 *  the attacher's transformer, which unified then calls as an attacher. */
function usePlugin(needed: MaybeRefOrGetter<boolean>, slot: Slot) {
  const plugin = shallowRef<Plugin | null>(slot.plugin);
  watch(
    () => toValue(needed),
    (want, _was, onCleanup) => {
      if (slot.plugin) {
        plugin.value = slot.plugin;
        return;
      }
      if (!want) return;
      let alive = true;
      // A failed chunk leaves the source text, which still reads.
      slot.loading ??= slot.load().then((p) => {
        slot.plugin = p;
      });
      void slot.loading.then(() => {
        if (alive && slot.plugin) plugin.value = slot.plugin;
      }).catch(() => {});
      onCleanup(() => {
        alive = false;
      });
    },
    { immediate: true },
  );
  return plugin;
}

// Streaming text arrives mid-token, so an unterminated fence would flip the
// whole tail into a code block for as long as it stays open. Close it for the
// render only; the source string is untouched.
function balanceFences(md: string) {
  const fences = md.match(/^```/gm)?.length ?? 0;
  return fences % 2 === 0 ? md : md + "\n```";
}

// A blank line ends a block, but not every one of them is safe to cut at: a
// list, a quote and an indented continuation all carry across one, so slicing
// there would render two lists where the author wrote one.
const CONTINUES = /^(\s|[-*+] |\d+[.)] |>)/;

// Nothing before a safe blank line can change as more text arrives, so each
// span between two of them is parsed once and then held. Splitting only the
// tail off was not enough: re-parsing the whole settled head every time a
// block closed cost 184ms on a long message.
function cutsOf(md: string): number[] {
  const cuts: number[] = [];
  let pos = 0;
  let cand = -1;
  let open = false;
  for (const line of md.split("\n")) {
    const next = pos + line.length + 1;
    const fence = line.startsWith("```");
    if (!open) {
      if (line.trim() === "") cand = next;
      else if (cand >= 0) {
        if (fence || !CONTINUES.test(line)) cuts.push(cand);
        cand = -1;
      }
    }
    if (fence) open = !open;
    pos = next;
  }
  return cuts;
}

interface HastNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

interface RenderCtx {
  local?: LocalRefs;
  images: boolean;
  live: boolean;
}

interface Pipeline {
  use(plugin: never, ...parameters: never[]): Pipeline;
  parse(input: string): unknown;
  runSync(tree: unknown): unknown;
}

// The mdast->hast step plugged into the unified chain; returning the new tree
// is how a transformer swaps the node domain. It is an attacher, because
// unified calls a bare function as one and would hand it no tree at all.
function mdastToHast() {
  return (tree: unknown) => toHast(tree as Parameters<typeof toHast>[0], { allowDangerousHtml: true });
}

function attach(pipeline: Pipeline, plugin: Plugin | null) {
  if (!plugin) return;
  if (Array.isArray(plugin)) pipeline.use(plugin[0] as never, plugin[1] as never);
  else pipeline.use(plugin as never);
}

function pipelineFor(ctx: RenderCtx, math: Plugin | null, emoji: Plugin | null, code: Plugin | null): Pipeline {
  const pipeline = unified() as unknown as Pipeline;
  pipeline
    .use(remarkParse as never)
    .use(remarkGfm as never, { singleTilde: false } as never)
    .use(remarkMath as never)
    .use(remarkTrimAutolink as never)
    .use(remarkCodeSpanLink as never)
    .use(remarkFileLinks as never, (ctx.local?.linkable ?? ctx.local?.path) as never)
    .use(remarkLocalImages as never, ctx.local?.path as never, ctx.local?.granted as never);
  attach(pipeline, emoji);
  pipeline.use(mdastToHast as never);
  // Order is load-bearing: raw parses HTML into nodes, sanitize prunes them, and
  // katex renders afterwards so its generated markup is not pruned in turn.
  pipeline.use(rehypeRaw as never).use(rehypeSanitize as never, schema as never);
  attach(pipeline, math);
  attach(pipeline, code);
  return pipeline;
}

function kebab(name: string) {
  return name.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
}

function attrName(key: string): string {
  if (/^data[A-Z]/.test(key)) return "data-" + kebab(key.slice(4));
  if (/^aria[A-Z]/.test(key)) return "aria-" + kebab(key.slice(4));
  return key;
}

function attrs(properties: Record<string, unknown> | undefined): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(properties ?? {})) {
    if (value === undefined || value === null) continue;
    if (key === "className") {
      const cls = Array.isArray(value) ? value.join(" ") : String(value);
      if (cls) out.class = cls;
      continue;
    }
    if (key === "htmlFor") {
      out.for = value;
      continue;
    }
    out[attrName(key)] = value;
  }
  return out;
}

const DROP = new Set(["script", "style"]);
const DIRECTED = new Set(["p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "blockquote", "td", "th"]);

// Each block takes its direction from its own first strong character, so a
// Persian or Arabic paragraph reads right to left beside an English one. Code
// keeps the document's direction: pre is not in this list.
function attrsOf(node: HastNode): Record<string, unknown> {
  const props = attrs(node.properties);
  if (node.tagName && DIRECTED.has(node.tagName)) props.dir = "auto";
  return props;
}

function renderNodes(nodes: HastNode[], ctx: RenderCtx): VNodeChild[] {
  return nodes.map((node) => renderNode(node, ctx));
}

function renderNode(node: HastNode, ctx: RenderCtx): VNodeChild {
  if (node.type === "text") return node.value ?? "";
  if (node.type === "raw") return node.value ?? "";
  if (node.type === "element") return renderElement(node, ctx);
  return null;
}

function textOf(node: unknown): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isVNode(node)) return textOf(node.children);
  return "";
}

function langOf(node: HastNode | undefined): string {
  if (!node || node.type !== "element") return "";
  const value = node.properties?.className;
  const cn = Array.isArray(value) ? value.join(" ") : typeof value === "string" ? value : "";
  return /language-([\w.+#-]+)/.exec(cn)?.[1] ?? "";
}

function renderLink(node: HastNode, ctx: RenderCtx): VNodeChild {
  const href = typeof node.properties?.href === "string" ? node.properties.href : undefined;
  const children = renderNodes(node.children ?? [], ctx);
  const local = ctx.local;
  const open = local && href ? local.link(href) : null;
  if (open && href) {
    return h("a", {
      href,
      title: local?.title?.(href),
      "data-action-click": "workbench.file",
      "data-target": local?.path?.(href) ?? href,
      "data-action-contextmenu": local?.menu ? "workbench.menu" : undefined,
      "data-action-keydown": local?.menu ? "workbench.menu" : undefined,
      onContextmenu: (event: MouseEvent) => local?.menu?.(event, href),
      onKeydown: (event: KeyboardEvent) => {
        if (event.key !== "ContextMenu" && !(event.key === "F10" && event.shiftKey)) return;
        local?.menu?.(event, href);
      },
      onClick: (event: MouseEvent) => {
        event.preventDefault();
        open();
      },
    }, children);
  }
  // An anchor alone points into a document this view does not number headings
  // for; followed as a link it would reload the window.
  if (local && href?.startsWith("#")) return h("a", null, children);
  return h("a", { href, target: "_blank", rel: "noreferrer noopener" }, children);
}

function renderImage(node: HastNode, ctx: RenderCtx): VNodeChild {
  const src = typeof node.properties?.src === "string" ? node.properties.src : undefined;
  const alt = typeof node.properties?.alt === "string" ? node.properties.alt : undefined;
  const title = typeof node.properties?.title === "string" ? node.properties.title : undefined;
  const at = ctx.local && src ? ctx.local.image(src) : null;
  if (ctx.images && src === OUTSIDE_IMAGE) {
    return h("span", { class: "md-noimg md-outside-image", title: t("chat.this_picture_is_outside_the") }, alt);
  }
  if (ctx.images && !at && src === undefined) return h("span", { class: "md-noimg" }, alt);
  return ctx.images ? h("img", { src: at || src, alt, title }) : h("span", { class: "md-noimg" }, alt);
}

function renderElement(node: HastNode, ctx: RenderCtx): VNodeChild {
  const tag = node.tagName;
  if (!tag || DROP.has(tag)) return null;
  if (tag === "a") return renderLink(node, ctx);
  if (tag === "img") return renderImage(node, ctx);
  const children = renderNodes(node.children ?? [], ctx);
  if (tag === "pre") {
    // The fence's own info string, as rehype leaves it on the <code>. Absent
    // for an indented block, which is why the tag is conditional rather than
    // defaulted to a guess at the language.
    return h(CodeBlock, { lang: langOf(node.children?.[0]), source: textOf(children), live: ctx.live }, { default: () => children });
  }
  if (tag === "table") {
    return h("div", { class: "md-tw" }, [h("table", attrsOf(node), children)]);
  }
  return h(tag, attrsOf(node), children);
}

function renderMarkdown(body: string, ctx: RenderCtx, math: Plugin | null, emoji: Plugin | null, code: Plugin | null): VNodeChild[] {
  const pipeline = pipelineFor(ctx, math, emoji, code);
  const tree = pipeline.runSync(pipeline.parse(body)) as HastNode;
  return renderNodes(tree.children ?? [], ctx);
}

/** File navigation for a workspace document or a conversation. */
export interface LocalRefs {
  file?(ref: FileRef): () => void;
  /** An action for href, or null when it is not a file in the workspace. */
  link(href: string): (() => void) | null;
  /** Where to load src from, or null to leave it as written. */
  image(src: string): string | null;
  path?(href: string): string | null;
  /** What a bare path in text may become a link to: the workspace's own file or one in a granted folder. */
  linkable?(href: string): string | null;
  /** The same for a local picture in a folder the user granted, spelled as GRANTED/<n>/<path>. */
  granted?(src: string): string | null;
  title?(href: string): string;
  menu?(event: MouseEvent | KeyboardEvent, href: string): void;
}

// live is the tail of a message still arriving; tail alone is only the last
// block, which a settled message has too.
interface MdBlockProps {
  src: string;
  math: Plugin | null;
  emoji: Plugin | null;
  code: Plugin | null;
  tail?: boolean;
  live?: boolean;
  local?: LocalRefs;
  images?: boolean;
}

const MdBlock = defineComponent({
  name: "MdBlock",
  props: {
    src: { type: String, required: true },
    math: { type: null, default: null },
    emoji: { type: null, default: null },
    code: { type: null, default: null },
    tail: { type: Boolean, default: false },
    live: { type: Boolean, default: false },
    local: { type: Object as PropType<LocalRefs> },
    images: { type: Boolean, default: true },
  },
  setup(props) {
    // Inside the block, so a settled block normalises once instead of per chunk.
    return () => {
      const body = normalizeMath(props.tail ? balanceFences(props.src) : props.src);
      return renderMarkdown(body, { local: props.local, images: props.images, live: props.live }, props.math, props.emoji, props.code);
    };
  },
}) as unknown as FunctionalComponent<MdBlockProps>;

/** images=false is for text nobody in the conversation wrote: a picture
 *  address in it is a request the reader never chose to make. */
export const Markdown = defineComponent({
  name: "Markdown",
  props: {
    text: { type: String, required: true },
    streaming: { type: Boolean, default: false },
    local: { type: Object as PropType<LocalRefs>, default: undefined },
    images: { type: Boolean, default: true },
  },
  setup(props) {
    const shown = useRevealed(() => props.text, () => props.streaming);
    const math = usePlugin(computed(() => MATH.test(props.text)), KATEX);
    const emoji = usePlugin(computed(() => EMOJI.test(props.text)), EMOJIS);
    const code = usePlugin(computed(() => FENCED.test(props.text)), HLJS);
    // Only a streamed message is split. Once it settles it parses whole again, so
    // nothing left in the transcript stands as a pile of separate documents.
    const cuts = computed(() => (props.streaming ? cutsOf(shown.value) : []));

    return () => {
      const parts: string[] = [];
      let at = 0;
      for (const cut of cuts.value) {
        parts.push(shown.value.slice(at, cut));
        at = cut;
      }
      const blocks: VNode[] = parts.map((src, i) => h(MdBlock, {
        key: `p${i}`, src, math: math.value, emoji: emoji.value, code: code.value, local: props.local, images: props.images,
      }));
      blocks.push(h(MdBlock, {
        key: "tail", src: shown.value.slice(at), math: math.value, emoji: emoji.value, code: code.value,
        tail: true, live: props.streaming, local: props.local, images: props.images,
      }));
      // Same rule the answer's own copy follows: while it is still arriving,
      // handing a listing over would hand over something that was never said.
      return h("div", { class: "md", "data-live": props.streaming ? "" : undefined }, [
        ...blocks,
        props.streaming ? h("span", { class: "caret" }) : null,
      ]);
    };
  },
});

export default Markdown;
</script>
