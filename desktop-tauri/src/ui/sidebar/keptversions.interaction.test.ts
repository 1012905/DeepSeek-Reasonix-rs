// @vitest-environment jsdom
//
// What this file proves: the two entries this wave added to the conversation
// menu reach the state they name. The kept entry exists only where the kernel
// reported earlier versions or recovery copies, and it spreads them inline as
// conversations of their own; the move entry hands the row to the landed
// composable, which asks the kernel and reads the tree again. The opening
// contract and the project's own new-session button are driven here too, from
// the column that wires them.
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
    remembered: true,
    sessions: [
      {
        path: "/work/site/a.jsonl",
        name: "a",
        title: "重构登录页",
        turns: 3,
        versions: [
          { path: "/work/site/a.v2.jsonl", name: "a.v2", turns: 4 },
          { path: "/work/site/a.v1.jsonl", name: "a.v1", turns: 2 },
        ],
        copies: [{ path: "/work/site/a.copy.jsonl", name: "a.copy", turns: 5 }],
      },
      { path: "/work/site/b.jsonl", name: "b", title: "修复构建脚本", turns: 1 },
    ],
  },
  {
    root: "/work/other",
    name: "other",
    remembered: true,
    sessions: [{ path: "/work/other/c.jsonl", name: "c", title: "别处的事", turns: 1 }],
  },
  // A folder the window happens to be in, not a project the person kept: a
  // conversation can travel to a remembered project, not to this.
  { root: "/work/here", name: "here", remembered: false, sessions: [] },
];

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  localStorage.removeItem("rx-sidebar-folds");
  document.body.querySelectorAll(".session-pop, .pickveil").forEach((el) => el.remove());
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

const settle = async (times = 6) => { for (let i = 0; i < times; i++) { await Promise.resolve(); await nextTick(); } };

const openMenu = async (wrapper: VueWrapper, path: string) => {
  await wrapper.get(`button[data-action="session.menu"][data-target="${path}"]`).trigger("click");
  await settle();
  const menu = document.body.querySelector<HTMLElement>(".session-pop");
  expect(menu, "the row's menu is drawn at the document root").toBeTruthy();
  return menu!;
};

const entry = (menu: HTMLElement, action: string) => menu.querySelector<HTMLElement>(`[data-action="${action}"]`);

const closed = () => expect(document.body.querySelector(".session-pop")).toBeNull();

const spread = async (wrapper: VueWrapper, path = "/work/site/a.jsonl") => {
  const menu = await openMenu(wrapper, path);
  entry(menu, "session.kept")!.click();
  await settle();
  closed();
};

describe("earlier versions and recovery copies", () => {
  it("offers the kept entry only where the kernel reported something kept", async () => {
    boot();
    const { wrapper } = rail();
    await settle();

    const bare = await openMenu(wrapper, "/work/site/b.jsonl");
    expect(entry(bare, "session.kept"), "a conversation with nothing kept offers no entry").toBeNull();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await settle();
    closed();

    const kept = await openMenu(wrapper, "/work/site/a.jsonl");
    const button = entry(kept, "session.kept")!;
    expect(button.textContent).toContain(t("session.earlier_versions_n", { n: 3 }));
    expect(button.getAttribute("aria-expanded")).toBe("false");
  });

  it("spreads the kept conversations inline under the row and folds them away again", async () => {
    boot();
    const { wrapper } = rail();
    await settle();
    expect(wrapper.findAll(".sesscopy")).toHaveLength(0);

    const menu = await openMenu(wrapper, "/work/site/a.jsonl");
    entry(menu, "session.kept")!.click();
    await settle();
    closed();

    const rows = wrapper.findAll(".sesscopy");
    expect(rows.map((row) => row.attributes("data-target"))).toEqual([
      "/work/site/a.v2.jsonl",
      "/work/site/a.v1.jsonl",
      "/work/site/a.copy.jsonl",
    ]);
    expect(rows[0].text()).toContain(t("session.earlier_version_n_turns", { n: 4 }));
    expect(rows[1].text()).toContain(t("session.earlier_version_n_turns", { n: 2 }));
    expect(rows[2].text()).toContain(t("session.recovery"));

    const again = await openMenu(wrapper, "/work/site/a.jsonl");
    expect(entry(again, "session.kept")!.getAttribute("aria-expanded")).toBe("true");
    entry(again, "session.kept")!.click();
    await settle();

    expect(wrapper.findAll(".sesscopy")).toHaveLength(0);
  });

  it("opens a kept conversation whole, naming that path", async () => {
    boot();
    const paths: string[] = [];
    const { wrapper } = rail({ props: { onOpenSession: (path: string) => paths.push(path) } });
    await settle();
    await spread(wrapper);

    await wrapper.get('.sesscopy[data-target="/work/site/a.v1.jsonl"]').trigger("click");

    expect(paths).toEqual(["/work/site/a.v1.jsonl"]);
  });

  it("asks before deleting a kept conversation, then deletes it through the kernel", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const remove = vi.spyOn(hub, "removeSession");
    await settle();
    await spread(wrapper);

    await wrapper.get('.sesscopy[data-target="/work/site/a.v1.jsonl"] [data-action="session.delete"]').trigger("click");
    await settle();

    expect(remove, "the first press only asks").not.toHaveBeenCalled();
    const ask = wrapper.get('[role="alertdialog"]');
    expect(ask.text()).toContain(t("session.delete_this_earlier_version"));
    // The row it replaced is gone, so the question takes the keyboard rather
    // than leaving it on the page behind.
    expect(document.activeElement).toBe(ask.get('[data-action="layer.dismiss"]').element);

    // Cancelling puts the row back and leaves the file alone.
    await ask.get('[data-action="layer.dismiss"]').trigger("click");
    await settle();
    expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
    expect(wrapper.find('.sesscopy[data-target="/work/site/a.v1.jsonl"]').exists()).toBe(true);
    expect(remove).not.toHaveBeenCalled();

    await wrapper.get('.sesscopy[data-target="/work/site/a.v1.jsonl"] [data-action="session.delete"]').trigger("click");
    await settle();
    await wrapper.get('[role="alertdialog"] [data-action="session.delete"]').trigger("click");
    await settle();

    expect(remove).toHaveBeenCalledExactlyOnceWith("/work/site/a.v1.jsonl");
    expect(reload).toHaveBeenCalled();
    expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
  });

  it("names a recovery copy by what it is when the delete is asked about", async () => {
    boot();
    const { wrapper } = rail();
    await settle();
    await spread(wrapper);

    await wrapper.get('.sesscopy[data-target="/work/site/a.copy.jsonl"] [data-action="session.delete"]').trigger("click");
    await settle();

    expect(wrapper.get('[role="alertdialog"]').text()).toContain(t("session.delete_this_recovery_copy"));
  });
});

describe("moving a conversation to another project", () => {
  it("offers only the remembered projects other than the source, moves, re-reads and closes", async () => {
    boot();
    const reload = vi.fn(async () => {});
    const { wrapper, hub } = rail({ reload });
    const moveSessionTo = vi.spyOn(hub, "moveSessionTo");
    await settle();

    const menu = await openMenu(wrapper, "/work/site/a.jsonl");
    const button = entry(menu, "session.move-to-start");
    expect(button, "the column offers the move-to entry").toBeTruthy();
    expect(button!.hasAttribute("disabled")).toBe(false);
    button!.click();
    await settle();
    closed();

    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog, "choosing the project opens the landed dialog").toBeTruthy();
    expect([...dialog!.querySelectorAll<HTMLInputElement>(".movess-row input")].map((input) => input.value)).toEqual(["/work/other"]);

    const form = dialog!.querySelector<HTMLFormElement>("form.movess")!;
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await settle(10);

    expect(moveSessionTo).toHaveBeenCalledExactlyOnceWith("/work/site/a.jsonl", "/work/other");
    expect(reload).toHaveBeenCalled();
    expect(document.body.querySelector('[role="dialog"]'), "the move landed, so the question is gone").toBeNull();
  });

  it("offers the move locked while a pane still holds the conversation", async () => {
    boot();
    const heldTree: TreeWorkspace[] = [
      { root: "/work/site", name: "site", remembered: true, sessions: [{ path: "/work/site/a.jsonl", name: "a", title: "重构登录页", runtimeId: "r1" }] },
      { root: "/work/other", name: "other", remembered: true, sessions: [] },
    ];
    const { wrapper } = rail({ tree: heldTree, props: { runs: { r1: { live: false } } } });
    await settle();

    const menu = await openMenu(wrapper, "/work/site/a.jsonl");
    const button = entry(menu, "session.move-to-start")!;
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(button.textContent).toContain(t("session.close_this_conversation_first_then"));
  });
});

describe("the column's opening contract", () => {
  it("marks the conversation being opened and says the column is busy", async () => {
    boot();
    const { wrapper } = rail({ props: { opening: "/work/site/a.jsonl", openingPending: true } });
    await settle();

    expect(wrapper.get('[data-target="/work/site/a.jsonl"]').attributes("data-opening")).toBe("");
    expect(wrapper.get('[data-target="/work/site/b.jsonl"]').attributes("data-opening")).toBeUndefined();
    expect(wrapper.get("aside.rail").attributes("aria-busy")).toBe("true");
    // A mark, not a selection: which pane is focused is still its own answer.
    expect(wrapper.get('[data-target="/work/site/a.jsonl"]').attributes("aria-selected")).toBe("false");
  });

  it("says nothing of the kind when no open is in flight", async () => {
    boot();
    const { wrapper } = rail();
    await settle();

    expect(wrapper.get("aside.rail").attributes("aria-busy")).toBeUndefined();
    expect(wrapper.findAll("[data-opening]")).toHaveLength(0);
  });

  it("starts a conversation in the project whose own button was pressed", async () => {
    boot();
    const onNewSession = vi.fn();
    const { wrapper } = rail({ props: { onNewSession } });
    await settle();

    await wrapper.get('button[data-action="session.new"][data-target="/work/site"]').trigger("click");
    expect(onNewSession).toHaveBeenCalledExactlyOnceWith("/work/site");

    await wrapper.get('button[data-action="session.new"]:not([data-target])').trigger("click");
    expect(onNewSession).toHaveBeenLastCalledWith();
  });
});
