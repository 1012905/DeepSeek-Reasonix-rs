// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { indexNote, titleOf } from "./ContentMatches.vue";
import { hitsFor, type Command } from "./Palette.vue";
import type { TreeWorkspace } from "../../port/hub";

const workspace = (root: string, name: string, titles: string[]): TreeWorkspace => ({
  root,
  name,
  sessions: titles.map((title) => ({ path: `${root}/${title}`, name: title, title })),
});

describe("the note a partial history index adds", () => {
  it("says nothing when every source answered", () => {
    expect(indexNote(undefined)).toBeNull();
    expect(indexNote({ state: "ready", indexed: 4, total: 4, failed: 0, partial: false })).toBeNull();
  });

  it("names the failed reason separately from the one still building", () => {
    expect(indexNote({ state: "ready", indexed: 3, total: 4, failed: 1, partial: true, reason: "failed" })?.reason).toBe("failed");
    expect(indexNote({ state: "indexing", indexed: 3, total: 4, failed: 0, partial: true, reason: "building" })?.reason).toBe("building");
  });

  it("counts what it has read only when there is a total to count against", () => {
    expect(indexNote({ state: "indexing", indexed: 3, total: 4, failed: 0, partial: true, reason: "building" })?.vars).toEqual({ indexed: 3, total: 4 });
    expect(indexNote({ state: "indexing", indexed: 0, total: 0, failed: 0, partial: true, reason: "building" })?.vars).toBeUndefined();
  });
});

describe("naming a content match", () => {
  const tree = [workspace("/w/a", "site", ["Deploy the site"])];

  it("prefers the title this column already has, so the same conversation is not named twice", () => {
    expect(titleOf({ path: "/w/a/Deploy the site", title: "stale title", id: "id-1" }, tree)).toBe("Deploy the site");
  });

  it("falls back to what the kernel's own hit carried, then to its id", () => {
    expect(titleOf({ path: "/w/b/hit", title: "From the index", id: "id-2" }, tree)).toBe("From the index");
    expect(titleOf({ path: "/w/b/hit", id: "id-3" }, tree)).toBe("id-3");
  });
});

describe("what the palette answers", () => {
  const commands: Command[] = [
    { id: "new", label: "Start a new session", icon: "plus", keywords: "new chat 新建", run: () => {} },
    { id: "settings", label: "Settings", icon: "settings", keywords: "preferences 设置", keys: "Ctrl ,", run: () => {} },
  ];
  const tree = [workspace("/w/a", "site", ["Deploy the site", "Rename a variable"]), workspace("/w/b", "api", ["Deploy the worker"])];

  it("asks both lists when the box is empty, actions first", () => {
    const hits = hitsFor(commands, tree, "");
    expect(hits.slice(0, 2).map((hit) => hit.group)).toEqual(["actions", "actions"]);
    expect(hits.filter((hit) => hit.group === "sessions").map((hit) => hit.label)).toEqual(["Deploy the site", "Rename a variable", "Deploy the worker"]);
  });

  it("finds a command by a word it does not print", () => {
    expect(hitsFor(commands, tree, "preferences").map((hit) => hit.key)).toEqual(["c:settings"]);
  });

  it("finds a conversation by its title, and every row of a project by the project's name", () => {
    expect(hitsFor(commands, tree, "worker").map((hit) => hit.label)).toEqual(["Deploy the worker"]);
    // A project's name is part of what each of its rows answers to: someone who
    // types it is asking about that project, not about one conversation in it.
    expect(hitsFor(commands, tree, "site").map((hit) => hit.label)).toEqual(["Deploy the site", "Rename a variable"]);
  });

  it("answers nothing rather than everything when the word matches none", () => {
    expect(hitsFor(commands, tree, "quarterly budget")).toEqual([]);
  });

  it("caps each group, so one long project cannot bury the other", () => {
    const big = [workspace("/w/big", "big", Array.from({ length: 40 }, (_, i) => `Row ${i}`))];
    expect(hitsFor([], big, "row").length).toBeLessThanOrEqual(6);
  });

  it("runs a command's own effect, and leaves a conversation to the caller", () => {
    const run = vi.fn();
    const hits = hitsFor([{ id: "x", label: "Do it", icon: "check", keywords: "", run }], tree, "do it");
    expect(hits[0].run).toBe(run);
    // A session row carries a path in its key instead: the opener belongs to the
    // window, and this list is built from data.
    expect(hitsFor([], tree, "").at(0)?.key).toBe("s:/w/a/Deploy the site");
  });
});
