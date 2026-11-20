// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/vue";
import { defineComponent, h, nextTick, type Component } from "vue";
import "./testkit";
import { t } from "../i18n";
import type { HubPort, TreeSession, TreeWorkspace } from "../port/hub";
import MoveSessionDialog from "./MoveSessionDialog.vue";
import { useSessionMoveTo } from "./useSessionMoveTo";

afterEach(cleanup);

const session = (path: string, extra: Partial<TreeSession> = {}): TreeSession => ({ path, name: path, ...extra });
const project = (root: string, remembered = true): TreeWorkspace => ({ root, name: root.slice(1), remembered, sessions: [] });

function hubOf(answer: { originalRemoved: boolean } | Error) {
  const moveSessionTo = vi.fn(async (_path: string, _target: string) => {
    if (answer instanceof Error) throw answer;
    return { path: "/dst/a.jsonl", ...answer };
  });
  return { hub: { moveSessionTo } as unknown as HubPort, moveSessionTo };
}

function mount(options: {
  tree: TreeWorkspace[];
  hub: HubPort;
  reload: () => Promise<unknown> | void;
  addProject?: () => void;
}) {
  let hook!: ReturnType<typeof useSessionMoveTo>;
  const view: Component = defineComponent({
    setup() {
      hook = useSessionMoveTo({
        hub: options.hub,
        tree: options.tree,
        reload: options.reload,
        label: (s) => s.title || s.name,
        addProject: options.addProject,
      });
      return () => (hook.dialog.value ? h(MoveSessionDialog, hook.dialog.value) : h("div"));
    },
  });
  const utils = render(view);
  return { hook, ...utils };
}

const rows = () => [...document.querySelectorAll<HTMLElement>(".movess-row input")].map((i) => (i as HTMLInputElement).value);
const moveButton = () => screen.getByRole("button", { name: t("session.move") }) as HTMLButtonElement;

describe("moving a conversation to another project", () => {
  it("opens with only the remembered projects other than the source", async () => {
    const { hook } = mount({
      tree: [project("/src"), project("/b"), project("/c", false)],
      hub: hubOf({ originalRemoved: true }).hub,
      reload: () => {},
    });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(rows()).toEqual(["/b"]);
  });

  it("moves to the chosen project, reads the tree again and closes", async () => {
    const { hub, moveSessionTo } = hubOf({ originalRemoved: true });
    const reload = vi.fn(async () => {});
    const { hook } = mount({ tree: [project("/src"), project("/b"), project("/c")], hub, reload });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    await fireEvent.click(screen.getAllByRole("radio")[1]);
    await fireEvent.click(moveButton());
    await waitFor(() => expect(moveSessionTo).toHaveBeenCalledWith("/src/a.jsonl", "/c"));
    expect(reload).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog and says so when the original could not be removed", async () => {
    const { hub, moveSessionTo } = hubOf({ originalRemoved: false });
    const reload = vi.fn(async () => {});
    const { hook } = mount({ tree: [project("/src"), project("/b")], hub, reload });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    await fireEvent.click(moveButton());
    await waitFor(() => expect(moveSessionTo).toHaveBeenCalledWith("/src/a.jsonl", "/b"));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(t("session.the_conversation_was_moved_but")));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("shows the kernel's refusal in its own words and stays open", async () => {
    const { hub, moveSessionTo } = hubOf(new Error("that project is gone"));
    const reload = vi.fn(async () => {});
    const { hook } = mount({ tree: [project("/src"), project("/b")], hub, reload });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    await fireEvent.click(moveButton());
    await waitFor(() => expect(moveSessionTo).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("that project is gone"));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(reload).not.toHaveBeenCalled();
    expect(moveButton().disabled).toBe(false);
  });

  it("closes without moving anything", async () => {
    const { hub, moveSessionTo } = hubOf({ originalRemoved: true });
    const { hook } = mount({ tree: [project("/src"), project("/b")], hub, reload: () => {} });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    await fireEvent.click(screen.getByRole("button", { name: t("common.cancel") }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(moveSessionTo).not.toHaveBeenCalled();
  });

  it("offers to add a project when there is nowhere else to go", async () => {
    const addProject = vi.fn();
    const { hub, moveSessionTo } = hubOf({ originalRemoved: true });
    const { hook } = mount({ tree: [project("/src")], hub, reload: () => {}, addProject });
    hook.request(session("/src/a.jsonl"), "/src");
    await nextTick();
    expect(screen.queryAllByRole("radio")).toHaveLength(0);
    await fireEvent.click(screen.getByRole("button", { name: t("session.add_a_project") }));
    expect(addProject).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(moveSessionTo).not.toHaveBeenCalled();
  });
});
