// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { treeQuery } from "../port/hub";
import { atCap, TREE_MAX, TREE_PAGE, unloaded, useTreeWindow } from "./treewindow";

describe("the tree window", () => {
  it("asks for one page of every folder until one is opened up", async () => {
    const win = useTreeWindow();
    expect(win.current()).toEqual({ limit: TREE_PAGE, limits: {}, scope: "", q: "" });
    await win.grow("/a", async () => {});
    await win.grow("/a", async () => {});
    expect(win.current().limits).toEqual({ "/a": TREE_PAGE * 3 });
  });

  it("never opens a folder past the cap", async () => {
    const win = useTreeWindow();
    for (let i = 0; i < 20; i++) await win.grow("/a", async () => {});
    expect(win.current().limits).toEqual({ "/a": TREE_MAX });
    expect(atCap(TREE_MAX)).toBe(true);
    expect(atCap(TREE_MAX - 1)).toBe(false);
  });

  it("takes a page back when the kernel could not give it", async () => {
    const win = useTreeWindow();
    await expect(win.grow("/a", async () => { throw new Error("down"); })).rejects.toThrow("down");
    expect(win.current().limits).toEqual({});
  });

  it("starts from the first page when the view changes, and says whether it changed", async () => {
    const win = useTreeWindow();
    await win.grow("/a", async () => {});
    expect(win.view({ scope: "archived", q: "" })).toBe(true);
    expect(win.view({ scope: "archived", q: "" })).toBe(false);
    expect(win.current()).toEqual({ limit: TREE_PAGE, limits: {}, scope: "archived", q: "" });
  });

  it("writes the window into the /tree query", () => {
    expect(treeQuery(undefined)).toBe("");
    const q = new URLSearchParams(treeQuery({ limit: 50, limits: { "/a b": 100 }, scope: "archived", q: " Plan " }));
    expect(q.get("limit")).toBe("50");
    expect(JSON.parse(q.get("limits")!)).toEqual({ "/a b": 100 });
    expect(q.get("scope")).toBe("archived");
    expect(q.get("q")).toBe("Plan");
    expect(new URLSearchParams(treeQuery({ limit: 50, scope: "", q: "" })).has("scope")).toBe(false);
  });
});

describe("what a folder has left to load", () => {
  const ws = (over: object) => ({ root: "/w", name: "w", sessions: [], ...over });
  it("takes the kernel's own remainder for the view", () => {
    expect(unloaded(ws({ remaining: 7, total: 100 }), 40)).toBe(7);
  });
  it("falls back to the total on a kernel that predates the view", () => {
    expect(unloaded(ws({ total: 100 }), 40)).toBe(60);
    expect(unloaded(ws({}), 40)).toBe(0);
  });
});