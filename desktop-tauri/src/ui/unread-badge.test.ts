// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref } from "vue";
import { afterEach, expect, it, vi } from "vitest";
import { host } from "../port/host";
import type { TreeWorkspace } from "../port/hub";
import { useUnreadBadge } from "./unread";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const book = (unread: boolean, archived = false): TreeWorkspace[] => [{ root: "/work", name: "work", sessions: [{ path: "/session", name: "task", unread, archived }] }];

function mount(tree: TreeWorkspace[], remote: Record<string, TreeWorkspace[] | null>) {
  const treeRef = ref(tree);
  const remoteRef = ref(remote);
  const utils = render(defineComponent({
    setup() {
      useUnreadBadge(treeRef, remoteRef);
      return () => h("div");
    },
  }));
  return { treeRef, remoteRef, ...utils };
}

it("includes remote unread completions and excludes archived sessions", async () => {
  const badge = vi.spyOn(host(), "setUnreadBadge").mockResolvedValue();
  const { remoteRef } = mount(book(true, true), { server: book(true) });
  expect(badge).toHaveBeenLastCalledWith(true);
  remoteRef.value = { server: book(false) };
  await nextTick();
  expect(badge).toHaveBeenLastCalledWith(false);
});

it("does not reissue an unchanged badge or break the app when a host refuses", async () => {
  const badge = vi.spyOn(host(), "setUnreadBadge").mockRejectedValue(new Error("unavailable"));
  const { treeRef, remoteRef } = mount(book(true), {});
  await Promise.resolve();
  treeRef.value = book(true);
  remoteRef.value = {};
  await nextTick();
  expect(badge).toHaveBeenCalledExactlyOnceWith(true);
});