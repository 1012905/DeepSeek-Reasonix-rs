// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { nextTick, ref } from "vue";
import type { AgentPort } from "../port/port";
import { grantedFilePath } from "./grantedfiles";
import { useConversationFiles } from "./useConversationFiles";

const imageURL = vi.fn((path: string, folder?: string) => `image:${path}${folder ? `@${folder}` : ""}`);

function portOf(extra: string[] = [], roots?: () => Promise<{ primary: string; extra: string[] }>): AgentPort {
  return {
    workspaceImageURL: imageURL,
    workspaceRoots: vi.fn(roots ?? (() => Promise.resolve({ primary: "/w", extra }))),
    revealsFiles: () => false,
    opensFiles: () => false,
  } as unknown as AgentPort;
}

const flush = async () => { await nextTick(); await Promise.resolve(); };

afterEach(() => {
  vi.restoreAllMocks();
  imageURL.mockClear();
});

describe("a link from the conversation", () => {
  it("asks the workbench for the file it links to, once, and reveals it", () => {
    const reveal = vi.fn();
    const files = useConversationFiles(portOf(), "/w", false, true, reveal, () => {});
    const open = files.refs.value.link("docs/notes.md");
    expect(open).toBeTypeOf("function");
    expect(files.request.value).toBeUndefined();
    open!();
    expect(files.request.value?.path).toBe("docs/notes.md");
    expect(reveal).toHaveBeenCalledTimes(1);
  });

  it("clears only the request its own action consumed", () => {
    const files = useConversationFiles(portOf(), "/w", false, true, () => {}, () => {});
    files.refs.value.link("docs/one.md")!();
    const first = files.request.value!;
    files.refs.value.link("docs/two.md")!();
    const second = files.request.value!;
    expect(second).not.toBe(first);
    first.consumed();
    expect(files.request.value).toBe(second);
    second.consumed();
    expect(files.request.value).toBeUndefined();
  });

  it("leaves nothing to replay when the pane moves to another port or root", async () => {
    const port = ref(portOf());
    const root = ref("/w");
    const files = useConversationFiles(port, root, false, true, () => {}, () => {});
    files.refs.value.link("docs/one.md")!();
    expect(files.request.value).toBeDefined();
    port.value = portOf();
    await nextTick();
    expect(files.request.value).toBeUndefined();
    files.refs.value.link("docs/one.md")!();
    root.value = "/other";
    await nextTick();
    expect(files.request.value).toBeUndefined();
  });
});

describe("a picture", () => {
  it("loads from a folder the user granted, named by that folder", async () => {
    const files = useConversationFiles(portOf(["/pictures"]), "/work", false, true, () => {}, () => {});
    await flush();
    const refs = files.refs.value;
    expect(refs.granted?.("/pictures/wall/moon.png")).toBe(".granted/0/wall/moon.png");
    expect(refs.image(".granted/0/wall/moon.png")).toBe("image:wall/moon.png@/pictures");
    expect(refs.image("/pictures/wall/moon.png")).toBe("image:wall/moon.png@/pictures");
    expect(refs.image("file:///pictures/wall/moon.png")).toBe("image:wall/moon.png@/pictures");
    expect(workspaceImageURLLast()).toEqual(["wall/moon.png", "/pictures"]);
    expect(refs.image("wall/moon.png")).toBe("image:wall/moon.png");
    expect(refs.image("/elsewhere/moon.png")).toBeNull();
    expect(refs.image("https://example.com/moon.png")).toBeNull();
  });

  it("says nothing when the grant list cannot be read", async () => {
    const fail = vi.fn();
    const files = useConversationFiles(portOf([], () => Promise.reject(new Error("no grants"))), "/w", false, true, () => {}, fail);
    await flush();
    expect(files.refs.value.image("/pictures/a.png")).toBeNull();
    expect(files.refs.value.linkable?.("/pictures/a.png")).toBeNull();
    expect(files.refs.value.granted?.("/pictures/a.png")).toBeNull();
    expect(fail).not.toHaveBeenCalled();
  });
});

describe("a path in a granted folder", () => {
  it("is linkable outside the workspace and opens by folder and relative path", async () => {
    const files = useConversationFiles(portOf(["/library"]), "/work", false, true, () => {}, () => {});
    await flush();
    const refs = files.refs.value;
    expect(refs.linkable?.("/library/notes/plan.md")).toBe(grantedFilePath("/library", "notes/plan.md"));
    expect(refs.linkable?.("file:///library/notes/plan.md")).toBe(grantedFilePath("/library", "notes/plan.md"));
    expect(refs.linkable?.("/elsewhere/notes/plan.md")).toBeNull();
    expect(refs.path?.("/library/notes/plan.md")).toBeNull();
    const open = refs.link("/library/notes/plan.md");
    expect(open).toBeTypeOf("function");
    open!();
    expect(files.request.value?.path).toBe(grantedFilePath("/library", "notes/plan.md"));
    refs.file?.({ path: "notes/plan.md", root: "/library" })!();
    expect(files.request.value?.path).toBe(grantedFilePath("/library", "notes/plan.md"));
  });

  it("hands out a new set of refs once the grants arrive, so a drawn conversation is redrawn", async () => {
    const files = useConversationFiles(portOf(["/library"]), "/work", false, true, () => {}, () => {});
    const before = files.refs.value;
    expect(before.linkable?.("/library/notes/plan.md")).toBeNull();
    await flush();
    expect(files.refs.value).not.toBe(before);
    expect(files.refs.value.linkable?.("/library/notes/plan.md")).toBe(grantedFilePath("/library", "notes/plan.md"));
  });
});

describe("the file menu", () => {
  const anchorAt = (left: number, bottom: number) => {
    const anchor = document.createElement("a");
    anchor.getBoundingClientRect = () => ({ left, bottom, top: bottom, right: left, width: 0, height: 0, x: left, y: bottom, toJSON: () => ({}) }) as DOMRect;
    return anchor;
  };
  const current = <E extends Event>(event: E, anchor: HTMLElement): E => {
    Object.defineProperty(event, "currentTarget", { configurable: true, value: anchor });
    return event;
  };

  it("opens where the pointer is and closes when the conversation leaves the screen", async () => {
    const shown = ref(true);
    const files = useConversationFiles(portOf(), "/w", false, shown, () => {}, () => {});
    const anchor = anchorAt(11, 33);
    const click = current(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }), anchor);
    files.refs.value.menu!(click, "docs/notes.md");
    expect(click.defaultPrevented).toBe(true);
    expect(files.menu.value?.path).toBe("docs/notes.md");
    expect(files.menu.value?.folder).toBe(false);
    expect(files.menu.value?.x).toBe(40);
    expect(files.menu.value?.y).toBe(50);
    expect(files.menu.value?.anchor).toBe(anchor);
    const first = files.menu.value!.id;
    files.refs.value.menu!(current(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 1, clientY: 2 }), anchor), "docs/notes.md");
    expect(files.menu.value!.id).toBeGreaterThan(first);
    shown.value = false;
    await nextTick();
    expect(files.menu.value).toBeUndefined();
  });

  it("anchors a press with no pointer at the row's lower-left corner", () => {
    const files = useConversationFiles(portOf(), "/w", false, true, () => {}, () => {});
    const anchor = anchorAt(11, 33);
    const key = current(new KeyboardEvent("keydown", { bubbles: true, cancelable: true }), anchor);
    files.refs.value.menu!(key, "docs/notes.md");
    expect(files.menu.value?.x).toBe(11);
    expect(files.menu.value?.y).toBe(33);
    const atZero = current(new MouseEvent("click", { bubbles: true, cancelable: true }), anchor);
    files.refs.value.menu!(atZero, "docs/notes.md");
    expect(files.menu.value?.x).toBe(11);
    expect(files.menu.value?.y).toBe(33);
  });

  it("leaves a press alone while text is selected or the conversation is hidden", () => {
    vi.spyOn(document, "getSelection").mockReturnValue({ isCollapsed: false } as unknown as Selection);
    const files = useConversationFiles(portOf(), "/w", false, true, () => {}, () => {});
    const selected = current(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }), anchorAt(0, 0));
    files.refs.value.menu!(selected, "docs/notes.md");
    expect(files.menu.value).toBeUndefined();
    expect(selected.defaultPrevented).toBe(false);
    vi.restoreAllMocks();
    const hidden = useConversationFiles(portOf(), "/w", false, false, () => {}, () => {});
    hidden.refs.value.menu!(current(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }), anchorAt(0, 0)), "docs/notes.md");
    expect(hidden.menu.value).toBeUndefined();
  });

  it("closes on request and when the pane changes workspace", async () => {
    const root = ref("/w");
    const files = useConversationFiles(portOf(), root, false, true, () => {}, () => {});
    files.refs.value.menu!(current(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }), anchorAt(0, 0)), "docs/notes.md");
    expect(files.menu.value).toBeDefined();
    files.closeMenu();
    expect(files.menu.value).toBeUndefined();
    files.refs.value.menu!(current(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }), anchorAt(0, 0)), "docs/notes.md");
    root.value = "/other";
    await nextTick();
    expect(files.menu.value).toBeUndefined();
  });
});

function workspaceImageURLLast(): unknown[] {
  return imageURL.mock.calls[imageURL.mock.calls.length - 1] ?? [];
}
