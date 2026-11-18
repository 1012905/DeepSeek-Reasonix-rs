import { describe, expect, it } from "vitest";
import { parse as parseSfc } from "vue/compiler-sfc";

// The monospaced face says "a machine produced this": a path, a command, an id,
// a column of figures that must not jump, a mark that has to hold its cell.
// Interface prose is none of those, and a window that sets its own sentences in
// it reads as a terminal rather than as something written.
//
// The judgement is structural, never a look at the words: a class is monospaced
// if a rule in app.css gives it var(--mono), and an element renders prose if its
// own children call t() or carry text. Nothing here reads what a string says.
// Only template markup is read: the render functions that build nodes in script
// carry no JSX for this scan to walk.
const CSS = import.meta.glob(["./app.css", "./find.css"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const SOURCES = import.meta.glob("../**/*.vue", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

// Classes that render prose and data through the same box. Splitting one means
// moving var(--mono) off the class and onto the value inside it, which is a
// change per call site rather than per rule — recorded so the count cannot grow
// while the work is outstanding. Never widen this to land a change.
// Classes that render prose and data through the same box. Splitting one means
// moving var(--mono) off the class and onto the value inside it, which is a
// change per call site rather than per rule — recorded so the count cannot grow
// while the work is outstanding. Never widen this to land a change; the third
// test here fails an entry the tree stopped using, so this list stays exactly
// as large as the debt it names.
const MIXED = new Set([
  "c", "cnt", "d", "dt", "fail", "k", "lb", "n", "nm", "ntimes",
  "pill", "r", "rt", "sc", "st", "sz", "tag", "u", "v",
]);

const TRANSLATORS = ["t", "tx", "plural", "tr"];

function monoClasses(css: string): Set<string> {
  const lines = css.split("\n");
  const out = new Set<string>();
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i].includes("var(--mono)")) continue;
    let sel: string | null = null;
    if (lines[i].includes("{")) sel = lines[i].split("{")[0];
    else
      for (let j = i - 1; j >= 0 && j > i - 12; j--)
        if (lines[j].includes("{")) { sel = lines[j].split("{")[0]; break; }
    if (!sel) continue;
    for (const one of sel.split(",")) {
      const last = one.trim().split(/\s+|>/).filter(Boolean).pop() ?? "";
      // A rule on ::before styles the pseudo-element's own content, not the text
      // the element carries.
      if (last.includes("::")) continue;
      for (const m of last.matchAll(/\.([A-Za-z][\w-]*)/g)) out.add(m[1]);
    }
  }
  return out;
}

type Landing = { cls: string; prose: boolean; data: boolean; where: string };

function landings(mono: Set<string>): Landing[] {
  const out: Landing[] = [];
  for (const [file, src] of Object.entries(SOURCES)) {
    const { descriptor, errors } = parseSfc(src, { filename: file });
    if (errors.length || !descriptor.template?.ast) continue;
    const visit = (node: unknown): void => {
      if (!node || typeof node !== "object") return;
      const n = node as { type?: number; props?: { type: number; name?: string; value?: { content: string } }[]; children?: unknown[]; branches?: unknown[]; loc?: { start: { line: number } } };
      if (n.type === 1) {
        const attr = n.props?.find((p) => p.type === 6 && p.name === "class");
        const hit = attr?.value ? attr.value.content.split(/\s+/).filter((c: string) => mono.has(c)) : [];
        if (hit.length) {
          let prose = false, data = false;
          for (const ch of n.children ?? []) {
            const child = ch as { type?: number; content?: string | { content: string } };
            if (child.type === 2 && (child.content as string).trim()) prose = true;
            else if (child.type === 5) {
              const expr = (child.content as { content: string }).content;
              if (TRANSLATORS.some((name) => new RegExp(`\\b${name}\\s*\\(`).test(expr))) prose = true;
              else data = true;
            }
          }
          if (prose || data) out.push({ cls: hit[0], prose, data, where: `${file.replace(/^\.\.\//, "")}:${n.loc?.start.line ?? 0}` });
        }
      }
      for (const ch of n.children ?? []) visit(ch);
      for (const b of n.branches ?? []) visit(b);
    };
    visit(descriptor.template.ast);
  }
  return out;
}

describe("monospace is for machine facts", () => {
  const mono = monoClasses(Object.values(CSS).join("\n"));
  const all = landings(mono);
  const byClass = new Map<string, Landing[]>();
  for (const l of all) byClass.set(l.cls, [...(byClass.get(l.cls) ?? []), l]);

  it("does not set interface prose in the monospaced face", () => {
    const offenders: string[] = [];
    for (const [cls, list] of byClass) {
      if (!list.some((l) => l.prose)) continue;      // renders data only
      if (list.some((l) => l.data)) continue;        // mixed, held below
      if (MIXED.has(cls)) continue;
      offenders.push(`.${cls} — ${list.filter((l) => l.prose).map((l) => l.where).join(", ")}`);
    }
    expect(offenders, "prose set in var(--mono); give the class var(--ui), or record it as mixed with its reason").toEqual([]);
  });

  it("does not grow the set of classes carrying both prose and data", () => {
    const grown = [...byClass]
      .filter(([cls, list]) => list.some((l) => l.prose) && list.some((l) => l.data) && !MIXED.has(cls))
      .map(([cls, list]) => `.${cls} — ${list[0].where}`);
    expect(grown, "a new class renders prose and data through one monospaced box; move var(--mono) onto the value").toEqual([]);
  });

  it("keeps every recorded exception in use", () => {
    const live = new Set([...byClass].filter(([, l]) => l.some((x) => x.prose)).map(([c]) => c));
    const stale = [...MIXED].filter((c) => !live.has(c));
    expect(stale, "listed as an exception but no longer renders prose in a monospaced class — drop it").toEqual([]);
  });
});
