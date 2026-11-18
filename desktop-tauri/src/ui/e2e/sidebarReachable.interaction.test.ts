// @vitest-environment jsdom
//
// What this file proves: the workspace column a person actually sees draws the
// projects and conversations it was handed, a press on a conversation row opens
// that conversation, typing in the search box narrows the list to what matched,
// and an empty machine still offers a way to open a first project. Sidebar's
// own modules are unit-tested; this mounts the column itself and drives it.
import { afterEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import "../testkit";
import Sidebar from "../Sidebar.vue";
import { MockHub } from "../../port/mock_hub";
import type { HubPort, TreeWorkspace } from "../../port/hub";
import { boot, STORAGE, t } from "../../i18n";

// A machine with one project holding two conversations, one of them unread.
const TREE: TreeWorkspace[] = [
  {
    root: "/work/site",
    name: "site",
    sessions: [
      { path: "/work/site/a.jsonl", name: "a", title: "重构登录页", turns: 3 },
      { path: "/work/site/b.jsonl", name: "b", title: "修复构建脚本", turns: 1, unread: true },
    ],
  },
];

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  localStorage.removeItem("rx-sidebar-folds");
  boot();
});

interface SideOptions {
  tree?: TreeWorkspace[];
  onOpenSession?: (path: string) => void;
  onNewSession?: () => void;
  onAddWorkspace?: () => void;
  onOpenSettings?: (at?: string) => void;
}

const rail = (opts: SideOptions = {}) => {
  const hub = new MockHub() as unknown as HubPort;
  const wrapper = mount(Sidebar, {
    props: {
      hub,
      tree: opts.tree ?? TREE,
      active: "",
      busy: false,
      onOpenSession: opts.onOpenSession ?? (() => {}),
      onNewSession: opts.onNewSession ?? (() => {}),
      onAddWorkspace: opts.onAddWorkspace ?? (() => {}),
      onArchiveWorkspace: () => {},
      reload: async () => {},
      onOpenSettings: opts.onOpenSettings,
      onError: () => {},
    },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return wrapper;
};

const settle = async (times = 3) => { for (let i = 0; i < times; i++) await nextTick(); };

describe("the workspace column a person actually uses", () => {
  it("draws the project and its conversations by the names the machine reported", async () => {
    boot();
    const wrapper = rail();
    await settle();

    const head = wrapper.get('.wsrow[data-target="/work/site"]');
    expect(head.attributes("role")).toBe("treeitem");
    expect(head.get(".wsname").text()).toBe("site");

    const rows = wrapper.findAll(".sessrow");
    expect(rows.length).toBe(2);
    expect(rows.map((row) => row.attributes("data-target"))).toEqual([
      "/work/site/a.jsonl",
      "/work/site/b.jsonl",
    ]);
    // The title, not the file name: a person reads a conversation, not a path.
    expect(wrapper.get('[data-target="/work/site/a.jsonl"] .sesstitle').text()).toBe("重构登录页");
    expect(wrapper.get('[data-target="/work/site/b.jsonl"] .sesstitle').text()).toBe("修复构建脚本");
    // The unread marker is on the row that is unread and nowhere else.
    expect(wrapper.get('[data-target="/work/site/b.jsonl"]').attributes("data-unread")).toBe("");
    expect(wrapper.get('[data-target="/work/site/a.jsonl"]').attributes("data-unread")).toBeUndefined();
  });

  it("opens the conversation whose row was pressed, naming that conversation's path", async () => {
    boot();
    const openedPaths: string[] = [];
    const wrapper = rail({ onOpenSession: (path) => openedPaths.push(path) });
    await settle();

    await wrapper.get('[data-target="/work/site/b.jsonl"]').trigger("click");

    expect(openedPaths).toEqual(["/work/site/b.jsonl"]);
  });

  it("narrows the tree to the conversations whose title holds what was typed", async () => {
    boot();
    const wrapper = rail();
    await settle();
    expect(wrapper.findAll(".sessrow").length).toBe(2);

    await wrapper.get(".wsfind input").setValue("登录");
    await settle();

    // One row left, and it is the one whose title contains the word.
    expect(wrapper.findAll(".sessrow").map((row) => row.attributes("data-target"))).toEqual(["/work/site/a.jsonl"]);
    // The project it belongs to is still drawn — a matching conversation whose
    // project vanished would be unreachable.
    expect(wrapper.find('.wsrow[data-target="/work/site"]').exists()).toBe(true);
  });

  it("says nothing matches, with a way back, when no conversation holds the word", async () => {
    boot();
    const wrapper = rail();
    await settle();

    await wrapper.get(".wsfind input").setValue("zzzz-no-such-conversation");
    await settle();

    expect(wrapper.findAll(".sessrow").length).toBe(0);
    const note = wrapper.get(".sessnote");
    expect(note.text()).toContain(t("common.nothing_matches"));
    const clear = note.findAll("button").find((b) => b.attributes("data-value") === "clear");
    expect(clear, "the note offers to clear the filter").toBeTruthy();

    await clear!.trigger("click");
    await settle();
    expect(wrapper.findAll(".sessrow").length).toBe(2);
  });

  it("guides a machine with no projects to open its first one", async () => {
    boot();
    const addWorkspace = vi.fn();
    const wrapper = rail({ tree: [], onAddWorkspace: addWorkspace });
    await settle();

    expect(wrapper.findAll(".sessrow").length).toBe(0);
    const note = wrapper.get(".sessnote");
    expect(note.text()).toContain(t("session.no_folders_yet"));
    const first = note.findAll("button").find((b) => b.text().includes(t("session.open_your_first_project")));
    expect(first, "the empty column offers to open a first project").toBeTruthy();

    await first!.trigger("click");
    expect(addWorkspace).toHaveBeenCalledTimes(1);
  });

  it("offers the settings column's own way into Settings from the quick-action palette", async () => {
    boot();
    const openSettings = vi.fn();
    const wrapper = rail({ onOpenSettings: openSettings });
    await settle();

    // Ctrl+K is the combination this column prints on the search box, so it is
    // the one that has to work.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true }));
    await settle();
    expect(wrapper.find(".wsfind input").attributes("data-action-keydown")).toBe("workspace.search");

    const settingsRow = [...document.querySelectorAll<HTMLElement>('[data-action="palette.pick"]')]
      .find((row) => row.textContent?.includes(t("common.settings")));
    expect(settingsRow, "the palette lists Settings").toBeTruthy();

    settingsRow!.click();
    await settle();

    expect(openSettings).toHaveBeenCalledTimes(1);
  });
});
