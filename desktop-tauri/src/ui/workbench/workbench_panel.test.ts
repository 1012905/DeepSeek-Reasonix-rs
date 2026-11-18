import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { compileScript, compileTemplate, parse } from "vue/compiler-sfc";
import { buildTree } from "../../state/gittree";
import type { WorkspaceChange } from "../../port/workspace";
import { absoluteWorkspacePath } from "./WorkbenchFileMenu.vue";
import { joinRepo, insideRepo } from "./WorkbenchGit.vue";
import { rowsOf, ROW_PAGE } from "./WorkbenchChanges.vue";
import { commitAge, fromFirstHunk, laneLine, laneWidth } from "./WorkbenchHistory.vue";

const at = (path: string) => fileURLToPath(new URL(path, import.meta.url));
const read = (path: string) => readFileSync(at(path), "utf8");

const change = (path: string, status = "M"): WorkspaceChange => ({ path, status });

describe("the workbench's panels", () => {
  it("spells a workspace path the way the platform does", () => {
    expect(absoluteWorkspacePath("C:\\work\\reasonix", "src/ui/App.vue")).toBe("C:\\work\\reasonix\\src\\ui\\App.vue");
    expect(absoluteWorkspacePath("/work/reasonix/", "src/ui/App.vue")).toBe("/work/reasonix/src/ui/App.vue");
    // A workspace that is not an absolute path is a remote one: no local
    // absolute path exists to copy.
    expect(absoluteWorkspacePath("~/work", "a.ts")).toBe("");
  });

  it("keeps a repository's own paths inside its own section", () => {
    expect(joinRepo("", "a.ts")).toBe("a.ts");
    expect(joinRepo("web", "a.ts")).toBe("web/a.ts");
    expect(insideRepo("", "a.ts")).toBe("a.ts");
    expect(insideRepo("web", "web/a.ts")).toBe("a.ts");
    // A path of another repository is none of this section's business.
    expect(insideRepo("web", "api/a.ts")).toBe("");
  });

  it("draws a change tree depth-first, closed folders and all their files left out", () => {
    const tree = buildTree([change("src/a.ts"), change("src/b/c.ts"), change("README.md")]);
    const rows = rowsOf(tree, new Set(["src/b"]), 100);
    // Folders come first at every level (buildTree's own order), and a closed
    // folder is one row: what is under it is not drawn at all.
    expect(rows.map((r) => `${r.depth}:${r.node.path}`)).toEqual(["0:src", "1:src/b", "1:src/a.ts", "0:README.md"]);
    // The page limit counts rows, so a huge tree is bounded rather than drawn.
    const open = rowsOf(tree, new Set(), 100).length;
    expect(open).toBeGreaterThan(2);
    expect(rowsOf(tree, new Set(), 2)).toHaveLength(2);
    expect(ROW_PAGE).toBeGreaterThan(100);
  });

  it("keeps the commit graph's lanes inside the width it reports", () => {
    expect(laneWidth(1)).toBe(16);
    expect(laneWidth(3)).toBe(40);
    expect(laneLine(8, 0, 8, 28)).toBe("M8 0V28");
    expect(laneLine(8, 0, 20, 28)).toContain("C8 14 20 14 20 28");
  });

  it("writes a commit's age as a count and a unit, never as a sentence", () => {
    const say = (key: string, vars?: Record<string, string | number>) => `${key}:${JSON.stringify(vars ?? {})}`;
    const now = 1_700_000_000_000;
    expect(commitAge(now / 1000, now, say)).toBe('workbench.history_now:{}');
    expect(commitAge(now / 1000 - 120, now, say)).toBe('workbench.history_min:{"n":2}');
    expect(commitAge(now / 1000 - 7200, now, say)).toBe('workbench.history_hour:{"n":2}');
    expect(commitAge(now / 1000 - 172_800, now, say)).toBe('workbench.history_day:{"n":2}');
  });

  it("drops the file header git prints above a commit's first hunk", () => {
    expect(fromFirstHunk("@@ -1 +1 @@\n-a\n+b")).toBe("@@ -1 +1 @@\n-a\n+b");
    expect(fromFirstHunk("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n")).toBe("@@ -1 +1 @@\n");
  });
});

// The panels are rendered by a container nobody has wired up yet; a template
// that does not compile is found here rather than on the day it is mounted.
const PANELS = [
  "./WorkbenchPanel.vue",
  "./WorkbenchHistory.vue",
  "./WorkbenchFileMenu.vue",
  "./WorkbenchGit.vue",
  "./WorkbenchChanges.vue",
  "../terminal/TerminalView.vue",
  "../terminal/TerminalLayer.vue",
  "../terminal/WorkbenchTerminals.vue",
  "../terminal/TerminalHead.vue",
  "../terminal/TerminalCloseAsk.vue",
];

describe("the workbench and terminal panels compile", () => {
  for (const panel of PANELS) {
    it(`${panel.replace("../", "")} is a well-formed single-file component`, () => {
      const source = read(panel);
      const filename = panel.split("/").at(-1)!;
      const { descriptor, errors } = parse(source, { filename });
      expect(errors).toEqual([]);
      const script = compileScript(descriptor, { id: filename });
      expect(descriptor.template).not.toBeNull();
      const template = compileTemplate({
        id: filename,
        filename,
        source: descriptor.template!.content,
        compilerOptions: { bindingMetadata: script.bindings },
      });
      expect(template.errors).toEqual([]);
    });
  }
});
