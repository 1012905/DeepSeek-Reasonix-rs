import { describe, expect, it } from "vitest";
import type { TreeSession, TreeWorkspace } from "../../port/hub";
import {
  SCOPES,
  allFolded,
  baseName,
  compactPath,
  filterTree,
  foldableRoots,
  foldedWith,
  isShut,
  matchesNeedle,
  orderSessions,
  parentName,
  recentRows,
  rowLabel,
  scopeCounts,
  workspaceOf,
} from "./workspaceTree";

const session = (path: string, extra: Partial<TreeSession> = {}): TreeSession => ({
  path,
  name: baseName(path),
  ...extra,
});

const workspace = (root: string, sessions: TreeSession[], extra: Partial<TreeWorkspace> = {}): TreeWorkspace => ({
  root,
  name: baseName(root),
  sessions,
  ...extra,
});

describe("naming rows", () => {
  it("falls back to the file name, and to a word rather than a timestamp for a pane with no turns yet", () => {
    expect(rowLabel(session("/s/2024-05-01T10-00.jsonl", { title: "Budget" }))).toBe("Budget");
    expect(rowLabel(session("/s/2024-05-01T10-00.jsonl"))).toBe("2024-05-01T10-00.jsonl");
    // A pane holds a conversation before its first turn is written; only the
    // file name exists then, and printing it would look like a timestamp.
    expect(rowLabel(session("/s/2024-05-01T10-00.jsonl", { runtimeId: "r1", turns: 0 }))).toBe("新会话");
  });

  it("reads both separators, and keeps the last two segments of a long path", () => {
    expect(baseName("/home/ada/projects/site")).toBe("site");
    expect(baseName("C:\\work\\api\\")).toBe("api");
    expect(parentName("/home/ada/projects/site")).toBe("projects");
    expect(parentName("/site")).toBe("");
    expect(compactPath("/home/ada/projects/site")).toBe("…/projects/site");
    expect(compactPath("/site")).toBe("/site");
  });

  it("matches on a word only when there is one, lowercased on both sides", () => {
    expect(matchesNeedle("Fix The Build", "")).toBe(true);
    expect(matchesNeedle("Fix The Build", "  ")).toBe(true);
    expect(matchesNeedle("Fix The Build", "build")).toBe(true);
    expect(matchesNeedle("Fix The Build", "tests")).toBe(false);
  });
});

describe("ordering a project's rows", () => {
  it("leads with the pinned ones and keeps the kernel's order inside each group", () => {
    const rows = [session("/a"), session("/b", { pinned: true }), session("/c"), session("/d", { pinned: true })];
    expect(orderSessions(rows).map((row) => row.path)).toEqual(["/b", "/d", "/a", "/c"]);
  });

  it("returns the kernel's own order untouched when nothing is pinned", () => {
    const rows = [session("/c"), session("/a"), session("/b")];
    expect(orderSessions(rows)).toEqual(rows);
  });

  it("does not write to the list it was handed", () => {
    const rows = Object.freeze([session("/a"), session("/b", { pinned: true })]);
    expect(orderSessions(rows).map((row) => row.path)).toEqual(["/b", "/a"]);
  });
});

describe("what the column draws for a search word and a scope", () => {
  const tree = [
    workspace("/w/site", [
      session("/w/site/1", { title: "Deploy the site", lastActiveAt: 30 }),
      session("/w/site/2", { title: "Rename a variable", lastActiveAt: 20, unread: true }),
      session("/w/site/3", { title: "Old plan", archived: true }),
    ]),
    workspace("/w/api", [session("/w/api/1", { title: "Deploy the worker", lastActiveAt: 10 })]),
  ];

  it("drops the projects a filter left with nothing, rather than heading an empty list", () => {
    const found = filterTree(tree, "rename", "all");
    expect(found.map((project) => project.root)).toEqual(["/w/site"]);
    expect(found[0].sessions.map((row) => row.path)).toEqual(["/w/site/2"]);
  });

  it("keeps every row of a project whose name answers the word", () => {
    const found = filterTree(tree, "api", "all");
    expect(found.map((project) => project.root)).toEqual(["/w/api"]);
    expect(found[0].sessions).toHaveLength(1);
  });

  it("finds an archived conversation by its words, and hides it otherwise", () => {
    expect(filterTree(tree, "old plan", "all").flatMap((project) => project.sessions.map((row) => row.path))).toEqual(["/w/site/3"]);
    expect(filterTree(tree, "", "all").flatMap((project) => project.sessions.map((row) => row.path))).not.toContain("/w/site/3");
    expect(filterTree(tree, "", "archived").flatMap((project) => project.sessions.map((row) => row.path))).toEqual(["/w/site/3"]);
  });

  it("answers the unread and driven scopes from the rows it has", () => {
    expect(filterTree(tree, "", "unread").flatMap((project) => project.sessions.map((row) => row.path))).toEqual(["/w/site/2"]);
    const held = [workspace("/w/live", [session("/w/live/1", { runtimeId: "r1" })])];
    expect(filterTree(held, "", "live").flatMap((project) => project.sessions.map((row) => row.path))).toEqual(["/w/live/1"]);
  });

  it("leaves the tree's own objects alone: filtering is a view, not an edit", () => {
    const before = tree[0].sessions.length;
    filterTree(tree, "deploy", "all");
    expect(tree[0].sessions).toHaveLength(before);
  });
});

describe("the numbers beside the scope choices", () => {
  const tree = [
    workspace("/w/a", [
      session("/w/a/1", { unread: true }),
      session("/w/a/2", { runtimeId: "r1", pinned: true }),
      session("/w/a/3", { archived: true }),
    ]),
    workspace("/w/b", [session("/w/b/1")]),
  ];

  it("counts what each choice would show", () => {
    expect(scopeCounts(tree)).toEqual({ all: 3, live: 1, waiting: 0, unread: 1, pinned: 1, archived: 1 });
  });

  it("offers no choice it cannot answer for", () => {
    // `waiting` needs the pane's live stream, which this column never receives.
    expect(SCOPES).not.toContain("waiting");
    expect(SCOPES).toContain("archived");
  });
});

describe("folding", () => {
  const tree = [
    workspace("/w/a", [session("/w/a/1")]),
    workspace("/w/b", []),
    workspace("/w/c", [], { total: 12 }),
  ];

  it("folds only the projects that have something in them", () => {
    expect(foldableRoots(tree)).toEqual(["/w/a", "/w/c"]);
  });

  it("is closed only when every foldable project is", () => {
    expect(allFolded(new Set(["/w/a"]), ["/w/a", "/w/c"])).toBe(false);
    expect(allFolded(new Set(["/w/a", "/w/c"]), ["/w/a", "/w/c"])).toBe(true);
    // Nothing to fold is not "everything folded": that would make the button
    // claim a state nobody can see.
    expect(allFolded(new Set(["/w/a"]), [])).toBe(false);
  });

  it("applies one decision to several projects without writing to the set it was handed", () => {
    const held = new Set(["/w/a", "/w/b"]);
    const shut = foldedWith(held, ["/w/a", "/w/c"], true);
    expect([...shut].sort()).toEqual(["/w/a", "/w/b", "/w/c"]);
    expect(held).toEqual(new Set(["/w/a", "/w/b"]));
    expect([...foldedWith(held, ["/w/a"], false)]).toEqual(["/w/b"]);
  });

  it("opens everything while a word is being searched, because a fold would hide what the word found", () => {
    expect(isShut(new Set(["/w/a"]), "/w/a", false)).toBe(true);
    expect(isShut(new Set(["/w/a"]), "/w/a", true)).toBe(false);
  });
});

describe("recent conversations", () => {
  const at = (n: number) => 1_700_000_000_000 + n;
  const tree = [
    workspace("/w/a", [session("/w/a/old", { lastActiveAt: at(1) }), session("/w/a/new", { lastActiveAt: at(9) })]),
    workspace("/w/b", [session("/w/b/mid", { lastActiveAt: at(5) }), session("/w/b/gone", { lastActiveAt: at(9), archived: true })]),
  ];

  it("orders by the last time something was said, newest first, across projects", () => {
    expect(recentRows(tree).map((row) => row.session.path)).toEqual(["/w/a/new", "/w/b/mid", "/w/a/old"]);
  });

  it("leaves out what was archived: this list is for carrying on", () => {
    expect(recentRows(tree).some((row) => row.session.path === "/w/b/gone")).toBe(false);
  });

  it("carries the project each row belongs to, so the row can say where it lives", () => {
    expect(recentRows(tree)[0].workspace.root).toBe("/w/a");
  });

  it("takes a limit of zero as the whole list", () => {
    expect(recentRows(tree, 0)).toHaveLength(3);
    expect(recentRows(tree, 2)).toHaveLength(2);
  });
});

describe("finding the project a conversation belongs to", () => {
  const tree = [workspace("/w/a", [session("/w/a/1")]), workspace("/w/b", [session("/w/b/1")])];

  it("answers by the row, not by the folder's own name", () => {
    expect(workspaceOf(tree, "/w/b/1")?.root).toBe("/w/b");
  });

  it("has no project for a path it does not list, or for none at all", () => {
    expect(workspaceOf(tree, "/w/c/1")).toBeUndefined();
    expect(workspaceOf(tree, undefined)).toBeUndefined();
  });
});
