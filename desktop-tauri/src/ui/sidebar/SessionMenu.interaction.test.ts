// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import "../testkit";
import SessionMenu from "./SessionMenu.vue";
import type { SessionMenuProps } from "./SessionMenu.vue";
import { boot, STORAGE, t } from "../../i18n";

const PATH = "/work/site/a.jsonl";
const opened: VueWrapper[] = [];

beforeEach(() => {
  boot();
});

afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  boot();
});

function draw(over: Partial<SessionMenuProps> = {}) {
  const on = {
    onRename: vi.fn(),
    onArchive: vi.fn(),
    onDelete: vi.fn(),
    onPin: vi.fn(),
    onAutoName: vi.fn(),
    onMarkRead: vi.fn(),
    onCopyInfo: vi.fn(),
    onExport: vi.fn(),
    onMove: vi.fn(),
    onPause: vi.fn(),
    onCloseConversation: vi.fn(),
    onBranch: vi.fn(),
  };
  const wrapper = mount(SessionMenu, {
    props: {
      label: "a",
      path: PATH,
      archived: false,
      held: false,
      live: false,
      pinned: false,
      exporting: false,
      autoNaming: false,
      canRename: true,
      canMove: { up: true, down: true },
      ...on,
      ...over,
    },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return { wrapper, on };
}

const items = (wrapper: VueWrapper) => wrapper.findAll('button[role="menuitem"]');

// The accessible name: an aria-label wins over the drawn text, as it does for
// the two export buttons whose visible text is only the format.
const named = (wrapper: VueWrapper, name: string) =>
  items(wrapper).filter((b) => (b.attributes("aria-label") ?? b.text()).includes(name));

const one = (wrapper: VueWrapper, name: string) => {
  const found = named(wrapper, name);
  expect(found, `one menuitem named ${name}`).toHaveLength(1);
  return found[0];
};

describe("the session row menu", () => {
  it("reports exactly one choice per entry, each carrying the session path", async () => {
    const { wrapper, on } = draw();
    expect(wrapper.get(".session-pop").attributes("aria-label")).toBe(t("common.session_actions"));
    for (const b of items(wrapper)) expect(b.attributes("data-target")).toBe(PATH);

    await one(wrapper, t("common.pin_session")).trigger("click");
    await one(wrapper, t("common.renamed")).trigger("click");
    await one(wrapper, t("session.auto_name")).trigger("click");
    await one(wrapper, t("common.archive_session")).trigger("click");
    await one(wrapper, t("common.delete_session")).trigger("click");

    expect(on.onPin).toHaveBeenCalledTimes(1);
    expect(on.onRename).toHaveBeenCalledTimes(1);
    expect(on.onAutoName).toHaveBeenCalledTimes(1);
    expect(on.onArchive).toHaveBeenCalledTimes(1);
    expect(on.onDelete).toHaveBeenCalledTimes(1);

    const live = draw({ held: true, live: true }).wrapper;
    for (const b of items(live)) expect(b.attributes("data-target")).toBe(PATH);
  });

  it("names the toggles by the state they will leave behind", () => {
    const { wrapper } = draw({ pinned: true, archived: true });
    expect(named(wrapper, t("common.unpin"))).toHaveLength(1);
    expect(named(wrapper, t("common.pin_session"))).toHaveLength(0);
    expect(named(wrapper, t("common.unarchive"))).toHaveLength(1);
    expect(one(wrapper, t("common.unarchive")).attributes("data-value")).toBe("restore");
  });

  it("exports in two formats, both locked while one export is in flight", async () => {
    const { wrapper, on } = draw();
    const md = one(wrapper, `${t("session.export_session")} · Markdown`);
    const json = one(wrapper, `${t("session.export_session")} · JSON`);
    expect(md.attributes("data-value")).toBe("md");
    expect(json.attributes("data-value")).toBe("json");
    expect(named(wrapper, t("session.export_session"))).toHaveLength(2);

    await md.trigger("click");
    expect(on.onExport).toHaveBeenLastCalledWith("md");
    await json.trigger("click");
    expect(on.onExport).toHaveBeenLastCalledWith("json");
    expect(on.onExport).toHaveBeenCalledTimes(2);

    const busy = draw({ exporting: true }).wrapper;
    for (const b of named(busy, t("session.export_session"))) {
      expect(b.attributes("disabled")).toBeDefined();
    }
  });

  it("shows naming in progress and cannot be started twice", () => {
    const { wrapper } = draw({ autoNaming: true });
    const naming = one(wrapper, t("session.naming"));
    expect(naming.attributes("disabled")).toBeDefined();
    expect(named(wrapper, t("session.auto_name"))).toHaveLength(0);
  });

  it("offers mark as read only when the caller supplies it, and reports the choice", async () => {
    const absent = draw({ onMarkRead: undefined }).wrapper;
    expect(named(absent, t("common.mark_as_read"))).toHaveLength(0);

    const { wrapper, on } = draw();
    await one(wrapper, t("common.mark_as_read")).trigger("click");
    expect(on.onMarkRead).toHaveBeenCalledTimes(1);
  });

  it("moves one place up or down, locking the direction with nowhere to go", async () => {
    const { wrapper, on } = draw();
    const up = one(wrapper, t("common.move_up"));
    const down = one(wrapper, t("common.move_down"));
    expect(up.attributes("data-value")).toBe("-1");
    expect(down.attributes("data-value")).toBe("1");
    await up.trigger("click");
    expect(on.onMove).toHaveBeenLastCalledWith(-1);
    await down.trigger("click");
    expect(on.onMove).toHaveBeenLastCalledWith(1);
    expect(on.onMove).toHaveBeenCalledTimes(2);

    const top = draw({ canMove: { up: false, down: true } }).wrapper;
    expect(one(top, t("common.move_up")).attributes("disabled")).toBeDefined();
    expect(one(top, t("common.move_down")).attributes("disabled")).toBeUndefined();

    const bottom = draw({ canMove: { up: true, down: false } }).wrapper;
    expect(one(bottom, t("common.move_up")).attributes("disabled")).toBeUndefined();
    expect(one(bottom, t("common.move_down")).attributes("disabled")).toBeDefined();

    const still = draw({ onMove: undefined }).wrapper;
    expect(named(still, t("common.move_up"))).toHaveLength(0);
    expect(named(still, t("common.move_down"))).toHaveLength(0);
  });

  it("copies session information only when the caller supplies it", async () => {
    const absent = draw({ onCopyInfo: undefined }).wrapper;
    expect(named(absent, t("session.copy_session_information"))).toHaveLength(0);

    const { wrapper, on } = draw();
    await one(wrapper, t("session.copy_session_information")).trigger("click");
    expect(on.onCopyInfo).toHaveBeenCalledTimes(1);
  });

  it("draws only the entries whose callbacks arrived", () => {
    const { wrapper } = draw({
      canRename: false,
      onPin: undefined,
      onAutoName: undefined,
      onMarkRead: undefined,
      onCopyInfo: undefined,
      onExport: undefined,
      onMove: undefined,
    });
    expect(items(wrapper).map((b) => b.attributes("data-action"))).toEqual([
      "session.archive",
      "session.delete",
    ]);
    expect(wrapper.findAll(".session-pop-divider")).toHaveLength(0);
  });

  it("says whether the conversation is running in its head", () => {
    const idle = draw().wrapper;
    expect(idle.get(".session-pop-head small").text()).toBe(`${t("common.done_2")} · ${t("session.local_workspace")}`);
    const running = draw({ held: true, live: true }).wrapper;
    expect(running.get(".session-pop-head small").text()).toBe(`${t("session.running")} · ${t("session.local_workspace")}`);
    const heldIdle = draw({ held: true, live: false }).wrapper;
    expect(heldIdle.get(".session-pop-head small").text()).toBe(`${t("common.done_2")} · ${t("session.local_workspace")}`);
  });

  it("offers pause only while a held conversation is running", async () => {
    const idle = draw({ held: true, live: false }).wrapper;
    expect(named(idle, t("session.pause_run"))).toHaveLength(0);
    const unheld = draw({ held: false, live: true }).wrapper;
    expect(named(unheld, t("session.pause_run"))).toHaveLength(0);
    const absent = draw({ held: true, live: true, onPause: undefined }).wrapper;
    expect(named(absent, t("session.pause_run"))).toHaveLength(0);

    const { wrapper, on } = draw({ held: true, live: true });
    const pause = one(wrapper, t("session.pause_run"));
    expect(pause.attributes("data-action")).toBe("session.pause");
    expect(pause.attributes("data-target")).toBe(PATH);
    await pause.trigger("click");
    expect(on.onPause).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("closes a held conversation only between turns", async () => {
    const unheld = draw({ held: false }).wrapper;
    expect(named(unheld, t("session.close_conversation"))).toHaveLength(0);

    const running = draw({ held: true, live: true }).wrapper;
    const locked = one(running, t("session.close_conversation"));
    expect(locked.attributes("disabled")).toBeDefined();
    expect(locked.get(".session-pop-why").text()).toBe(t("session.running_pause_it_before_closing"));

    const { wrapper, on } = draw({ held: true, live: false });
    const close = one(wrapper, t("session.close_conversation"));
    expect(close.attributes("disabled")).toBeUndefined();
    expect(close.find(".session-pop-why").exists()).toBe(false);
    await close.trigger("click");
    expect(on.onCloseConversation).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("branches from the latest reply only for a held conversation", async () => {
    const unheld = draw({ held: false }).wrapper;
    expect(named(unheld, t("session.branch_from_the_latest_reply"))).toHaveLength(0);
    const absent = draw({ held: true, onBranch: undefined }).wrapper;
    expect(named(absent, t("session.branch_from_the_latest_reply"))).toHaveLength(0);

    const solo = draw({ held: true, onCopyInfo: undefined, onExport: undefined }).wrapper;
    expect(one(solo, t("session.branch_from_the_latest_reply")).attributes("data-action")).toBe("session.branch");

    const { wrapper, on } = draw({ held: true });
    const branch = one(wrapper, t("session.branch_from_the_latest_reply"));
    expect(branch.attributes("data-action")).toBe("session.branch");
    await branch.trigger("click");
    expect(on.onBranch).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("locks archive while a held conversation is running", () => {
    const running = draw({ held: true, live: true }).wrapper;
    expect(one(running, t("common.archive_session")).attributes("disabled")).toBeDefined();
    const idle = draw({ held: true, live: false }).wrapper;
    expect(one(idle, t("common.archive_session")).attributes("disabled")).toBeUndefined();
    const unheld = draw({ held: false, live: true }).wrapper;
    expect(one(unheld, t("common.archive_session")).attributes("disabled")).toBeUndefined();
  });

  it("walks the enabled entries with the arrow keys and leaves on Escape", async () => {
    const { wrapper } = draw({ canMove: { up: false, down: true } });
    const enabled = wrapper.findAll('button[role="menuitem"]:not(:disabled)');
    (wrapper.vm as unknown as { focus: () => void }).focus();
    expect(document.activeElement).toBe(enabled[0].element);

    const root = wrapper.get(".session-pop");
    await root.trigger("keydown", { key: "ArrowDown" });
    expect(document.activeElement).toBe(enabled[1].element);
    await root.trigger("keydown", { key: "ArrowDown" });
    expect(document.activeElement).toBe(enabled[2].element);
    await root.trigger("keydown", { key: "ArrowUp" });
    expect(document.activeElement).toBe(enabled[1].element);
    await root.trigger("keydown", { key: "End" });
    expect(document.activeElement).toBe(enabled[enabled.length - 1].element);
    await root.trigger("keydown", { key: "Home" });
    expect(document.activeElement).toBe(enabled[0].element);

    await root.trigger("keydown", { key: "Escape" });
    expect(wrapper.emitted("close")).toHaveLength(1);
    await root.trigger("keydown", { key: "Tab" });
    expect(wrapper.emitted("close")).toHaveLength(2);
  });

  it("closes itself around whichever entry was chosen", async () => {
    const { wrapper, on } = draw();
    await one(wrapper, t("common.delete_session")).trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
    expect(on.onDelete).toHaveBeenCalledTimes(1);
  });
});