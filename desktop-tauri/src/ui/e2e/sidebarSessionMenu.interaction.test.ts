// @vitest-environment jsdom
//
// What this file proves: the conversation menu the column raises routes each
// choice to the kernel mutation that owns it, asks the window for the tree
// again, and closes — and an entry appears only when its action is wired. The
// menu component's own choices are unit-tested beside it; this drives the
// column that wires them, where closing must not empty the target the choice
// still needs.
import { afterEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import "../testkit";
import Sidebar from "../Sidebar.vue";
import { MockHub } from "../../port/mock_hub";
import type { HubPort, TreeWorkspace } from "../../port/hub";
import { boot, STORAGE, t } from "../../i18n";

const TREE: TreeWorkspace[] = [
  {
    root: "/work/site",
    name: "site",
    sessions: [
      { path: "/work/site/a.jsonl", name: "a", title: "第一个", turns: 3 },
      { path: "/work/site/b.jsonl", name: "b", title: "第二个", turns: 1, unread: true },
    ],
  },
];

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  localStorage.removeItem("rx-sidebar-folds");
  document.body.querySelectorAll(".session-pop").forEach((el) => el.remove());
  vi.restoreAllMocks();
  boot();
});

interface Options {
  tree?: TreeWorkspace[];
  reload?: () => Promise<void>;
  props?: Record<string, unknown>;
}

const rail = (opts: Options = {}) => {
  const hub = new MockHub() as unknown as HubPort;
  const wrapper = mount(Sidebar, {
    props: {
      hub,
      tree: opts.tree ?? TREE,
      active: "",
      busy: false,
      onOpenSession: () => {},
      onNewSession: () => {},
      onAddWorkspace: () => {},
      onArchiveWorkspace: () => {},
      reload: opts.reload ?? (async () => {}),
      onError: () => {},
      onRenameSession: () => {},
      onArchiveSession: () => {},
      onDeleteSession: () => {},
      ...opts.props,
    },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return { wrapper, hub };
};

const settle = async (times = 5) => { for (let i = 0; i < times; i++) await nextTick(); };

const openMenu = async (wrapper: VueWrapper, path: string) => {
  await wrapper.get(`button[data-action="session.menu"][data-target="${path}"]`).trigger("click");
  await settle();
  const menu = document.body.querySelector<HTMLElement>(".session-pop");
  expect(menu, "the row's menu is drawn at the document root").toBeTruthy();
  return menu!;
};

const entry = (menu: HTMLElement, action: string, value?: string) =>
  [...menu.querySelectorAll<HTMLElement>(`[data-action="${action}"]`)]
    .find((el) => value === undefined || el.getAttribute("data-value") === value);

const closed = () => expect(document.body.querySelector(".session-pop")).toBeNull();

describe("the conversation menu the column wires", () => {
  it("offers the actions the column runs and the ones the window wired", async () => {
    boot();
    const { wrapper } = rail();
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    for (const action of ["session.pin", "session.auto-name", "session.copy-info", "session.rename-start", "session.archive", "session.delete"]) {
      expect(entry(menu, action), action).toBeTruthy();
    }
    expect(entry(menu, "session.export", "md")).toBeTruthy();
    expect(entry(menu, "session.export", "json")).toBeTruthy();
    expect(entry(menu, "session.move", "1")).toBeTruthy();
    // A row that is not unread offers no mark-as-read.
    expect(entry(menu, "session.mark-read")).toBeUndefined();
  });

  it("pins the conversation, asks for the tree again, and closes", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const pin = vi.spyOn(hub, "pinSession");
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.pin")!.click();
    await settle();

    expect(pin).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl", true);
    expect(reload).toHaveBeenCalled();
    closed();
  });

  it("marks an unread conversation read and closes", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const read = vi.spyOn(hub, "markSessionRead");
    await settle();
    const menu = await openMenu(wrapper, "/work/site/b.jsonl");

    const mark = entry(menu, "session.mark-read");
    expect(mark, "an unread row offers mark-as-read").toBeTruthy();
    mark!.click();
    await settle();

    expect(read).toHaveBeenCalledExactlyOnceWith("/work/site/b.jsonl");
    expect(reload).toHaveBeenCalled();
    closed();
  });

  it("moves the conversation down within its project and closes", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const move = vi.spyOn(hub, "moveSession");
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.move", "1")!.click();
    await settle();

    expect(move).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl", 1);
    expect(reload).toHaveBeenCalled();
    closed();
  });

  it("names the conversation from the model, keeps the title, and closes", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const name = vi.spyOn(hub, "autoNameSession");
    const rename = vi.spyOn(hub, "renameSession");
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.auto-name")!.click();
    await settle();

    expect(name).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl");
    expect(rename).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl", "最近的聊天话题");
    expect(reload).toHaveBeenCalled();
    closed();
  });

  it("exports the conversation as Markdown and closes", async () => {
    boot();
    const { wrapper, hub } = rail();
    const exported = vi.spyOn(hub, "exportSession");
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.export", "md")!.click();
    await settle();

    expect(exported).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl");
    closed();
  });

  it("still archives through the window's own action and closes", async () => {
    boot();
    const archive = vi.fn();
    const { wrapper } = rail({ props: { onArchiveSession: archive } });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.archive")!.click();
    await settle();

    expect(archive).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl");
    closed();
  });

  it("offers no rename where the window wired none, but keeps the column's own actions", async () => {
    boot();
    const { wrapper } = rail({ props: { onRenameSession: undefined } });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    expect(entry(menu, "session.rename-start")).toBeUndefined();
    expect(entry(menu, "session.pin")).toBeTruthy();
    expect(entry(menu, "session.export", "json")).toBeTruthy();
  });

  it("carries the row's pinned state into the menu's toggle", async () => {
    boot();
    const { wrapper } = rail({
      tree: [{ root: "/work/site", name: "site", sessions: [{ path: "/work/site/a.jsonl", name: "a", title: "第一个", pinned: true }] }],
    });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    expect(entry(menu, "session.pin")!.textContent).toContain(t("common.unpin"));
  });
});

// The runtime-gated entries read the row's own pane, not the window's focus:
// `active` stays empty here, so a conversation a background pane holds and runs
// must still offer pause and lock close/archive. A focused-only reading would
// treat every one of these rows as idle and unheld.
describe("the menu's runtime-gated entries", () => {
  const heldTree = (runtimeId?: string): TreeWorkspace[] => [
    { root: "/work/site", name: "site", sessions: [{ path: "/work/site/a.jsonl", name: "a", title: "第一个", turns: 3, ...(runtimeId ? { runtimeId } : {}) }] },
  ];

  it("pauses a running conversation and locks close/archive while it runs", async () => {
    boot();
    const onPauseSession = vi.fn();
    const { wrapper } = rail({
      tree: heldTree("r1"),
      props: { runs: { r1: { live: true } }, onPauseSession, onCloseSession: vi.fn(), onBranchSession: vi.fn(async () => {}) },
    });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    expect(entry(menu, "session.pause"), "a live held row offers pause").toBeTruthy();
    expect(entry(menu, "session.close")!.hasAttribute("disabled")).toBe(true);
    expect(entry(menu, "session.archive")!.hasAttribute("disabled")).toBe(true);

    entry(menu, "session.pause")!.click();
    await settle();
    expect(onPauseSession).toHaveBeenCalledExactlyOnceWith("r1");
    closed();
  });

  it("offers close and archive once the held conversation is idle, and no pause", async () => {
    boot();
    const onCloseSession = vi.fn();
    const { wrapper } = rail({
      tree: heldTree("r1"),
      props: { runs: { r1: { live: false } }, onPauseSession: vi.fn(), onCloseSession, onBranchSession: vi.fn(async () => {}) },
    });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    expect(entry(menu, "session.pause")).toBeUndefined();
    expect(entry(menu, "session.close")!.hasAttribute("disabled")).toBe(false);
    expect(entry(menu, "session.archive")!.hasAttribute("disabled")).toBe(false);

    entry(menu, "session.close")!.click();
    await settle();
    expect(onCloseSession).toHaveBeenCalledExactlyOnceWith("r1");
    closed();
  });

  it("branches a held conversation, named for the row it cuts from", async () => {
    boot();
    const onBranchSession = vi.fn(async () => {});
    const { wrapper } = rail({
      tree: heldTree("r1"),
      props: { runs: { r1: { live: false } }, onPauseSession: vi.fn(), onCloseSession: vi.fn(), onBranchSession },
    });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    entry(menu, "session.branch")!.click();
    await settle();
    expect(onBranchSession).toHaveBeenCalledExactlyOnceWith("r1", `第一个 · ${t("common.branch")}`);
    closed();
  });

  it("offers no runtime entry for a conversation no pane holds", async () => {
    boot();
    const { wrapper } = rail({
      tree: heldTree(),
      props: { onPauseSession: vi.fn(), onCloseSession: vi.fn(), onBranchSession: vi.fn(async () => {}) },
    });
    await settle();
    const menu = await openMenu(wrapper, "/work/site/a.jsonl");

    expect(entry(menu, "session.pause")).toBeUndefined();
    expect(entry(menu, "session.close")).toBeUndefined();
    expect(entry(menu, "session.branch")).toBeUndefined();
    expect(entry(menu, "session.archive")!.hasAttribute("disabled")).toBe(false);
  });
});