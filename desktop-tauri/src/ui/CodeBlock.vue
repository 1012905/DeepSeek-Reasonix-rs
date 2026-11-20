<script lang="ts">
import { computed, defineComponent, h, ref, type VNodeChild } from "vue";
import { t } from "../i18n";
import CopyButton from "./CopyButton.vue";
import StudioIcon from "./StudioIcon.vue";
import { splitLines } from "./codelines";

// Past this many lines a settled block opens folded to FOLDED_LINES, so one long
// file in a reply does not push the answer around it off the screen.
const FOLD_OVER = 18;
export const FOLDED_LINES = 12;

// The file a downloaded block is saved as. A fence names a language, not a file
// type; the ones missing here save as .txt rather than under a guessed suffix.
const EXT: Record<string, string> = {
  go: "go", python: "py", py: "py", javascript: "js", js: "js", typescript: "ts", ts: "ts",
  tsx: "tsx", jsx: "jsx", html: "html", css: "css", json: "json", bash: "sh", sh: "sh",
  shell: "sh", zsh: "sh", powershell: "ps1", ps1: "ps1", sql: "sql", yaml: "yaml", yml: "yaml",
  toml: "toml", markdown: "md", md: "md", rust: "rs", rs: "rs", java: "java", c: "c",
  cpp: "cpp", "c++": "cpp", csharp: "cs", cs: "cs", ruby: "rb", rb: "rb", php: "php",
  xml: "xml", kotlin: "kt", swift: "swift", lua: "lua", dockerfile: "Dockerfile",
};

export function extensionFor(lang: string): string {
  return EXT[lang.toLowerCase()] ?? "txt";
}

function download(source: string, lang: string) {
  const ext = extensionFor(lang);
  const name = ext === "Dockerfile" ? ext : `snippet.${ext}`;
  const url = URL.createObjectURL(new Blob([source], { type: "text/plain;charset=utf-8" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** A fenced block in rendered markdown. `live` is a block still arriving, which
 *  never folds: its height changing under the reader is the stream, not a choice. */
export const CodeBlock = defineComponent({
  name: "CodeBlock",
  props: {
    lang: { type: String, required: true },
    source: { type: String, required: true },
    live: { type: Boolean, default: false },
  },
  setup(props, { slots }) {
    const open = ref(false);
    const lines = computed(() => props.source.replace(/\n+$/, "").split("\n").length);
    const foldable = computed(() => !props.live && lines.value > FOLD_OVER);
    const folded = computed(() => foldable.value && !open.value);

    const rows = (): VNodeChild[][] => {
      const all = splitLines(slots.default?.() ?? []);
      while (all.length > lines.value && all[all.length - 1].length === 0) all.pop();
      return all;
    };

    return () => h("div", { class: "code-wrap", "data-lang": props.lang || undefined, "data-folded": folded.value ? "" : undefined }, [
      h("div", { class: "code-hd" }, [
        h("span", { class: "code-lang" }, props.lang || t("chat.code")),
        h("span", { class: "code-lines" }, t("chat.n_lines", { n: lines.value })),
        h("span", { class: "code-acts" }, [
          !props.live && h("button", {
            type: "button", class: "code-act", "data-action": "code.download",
            title: t("chat.download_as_file"), "aria-label": t("chat.download_as_file"),
            onClick: () => download(props.source, props.lang),
          }, [h(StudioIcon, { name: "download" })]),
          !props.live && h(CopyButton, { text: props.source, iconOnly: true, className: "code-act", label: t("chat.copy_this_code") }),
        ]),
      ]),
      h("div", { class: "code-body", style: { "--ln-w": `${String(lines.value).length + 1}ch` } }, [
        h("pre", { class: "term" }, rows().map((row, i) => h("span", { class: "code-ln", key: i }, [
          h("i", { class: "code-no", "aria-hidden": "true" }, String(i + 1)),
          h("span", { class: "code-tx" }, row.length ? row : "\n"),
        ]))),
      ]),
      foldable.value && h("button", {
        type: "button", class: "code-more", "data-action": "code.fold", "aria-expanded": open.value,
        onClick: () => { open.value = !open.value; },
      }, [
        h(StudioIcon, { name: "chevron" }),
        h("span", null, open.value ? t("common.collapse") : t("chat.show_all_n_lines", { n: lines.value })),
      ]),
    ]);
  },
});

export default CodeBlock;
</script>
