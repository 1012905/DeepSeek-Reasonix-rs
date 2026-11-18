// @vitest-environment jsdom
import { cleanup, render, waitFor } from "@testing-library/vue";
import { defineComponent, h, nextTick, ref, shallowRef, type Ref } from "vue";
import { afterEach, expect, it, vi } from "vitest";
import { HttpError, type AgentPort } from "../port/port";
import type { HistoryPage } from "../port/session";
import type { SessionEvent } from "../state/session";
import { useHistoryPages, useHistorySearch, type HistoryPaging } from "./useHistoryPages";

afterEach(cleanup);
const page = (snapshot = "one", cursor = "older"): HistoryPage => ({ snapshot, start: cursor ? 1 : 0, end: 2, total: 2, hasMoreBefore: !!cursor, nextCursor: cursor,
  messages: [{ role: "user", content: snapshot, msgIndex: 1 }], toolContext: [] });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }

type Port = Pick<AgentPort, "history" | "historyPage">;

function mount(port: Port, dispatch: (event: SessionEvent) => void) {
  const bound = shallowRef<Port>(port);
  let hook!: ReturnType<typeof useHistoryPages>;
  const utils = render(defineComponent({
    setup() {
      hook = useHistoryPages(bound, dispatch);
      return () => h("div");
    },
  }));
  const bind = async (next: Port) => { bound.value = next; await nextTick(); };
  return { hook, bind, ...utils };
}

function mountSearch(port: Port, dispatch: (event: SessionEvent) => void, query: Ref<string>) {
  let hook!: ReturnType<typeof useHistoryPages>;
  const utils = render(defineComponent({
    setup() {
      hook = useHistoryPages(port, dispatch);
      useHistorySearch(hook.paging, true, query);
      return () => h("div");
    },
  }));
  return { hook, ...utils };
}

it("loads a bounded tail and coalesces overlapping older-page requests", async () => {
  const older = deferred<HistoryPage>();
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockResolvedValueOnce(page()).mockReturnValueOnce(older.promise) };
  const dispatch = vi.fn();
  const { hook } = mount(port, dispatch);
  await hook.load();
  expect(port.history).not.toHaveBeenCalled();
  const one = hook.paging.more();
  expect(hook.paging.more()).toBe(one);
  expect(port.historyPage).toHaveBeenCalledTimes(2);
  older.resolve({ ...page("one", ""), start: 0, end: 1 });
  await one;
  await nextTick();
  expect(dispatch.mock.calls.at(-1)?.[0].kind).toBe("__history_prepend");
  expect(hook.paging.hasMore.value).toBe(false);
});

it("falls back only when paging is unsupported, and remembers that capability", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockRejectedValue(new HttpError(404, "missing")) };
  const { hook } = mount(port, vi.fn());
  await hook.load();
  await hook.load();
  expect(port.historyPage).toHaveBeenCalledTimes(1);
  expect(port.history).toHaveBeenCalledTimes(2);
});

it("does not turn a failed page request into an expensive full-history request", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockRejectedValue(new HttpError(503, "unavailable")) };
  const { hook } = mount(port, vi.fn());
  await expect(hook.load()).rejects.toBeInstanceOf(HttpError);
  expect(port.history).not.toHaveBeenCalled();
  expect(hook.paging.failed.value).toBe(true);
});

it("supports ports without paging and servers explicitly refusing the optional capability", async () => {
  const history = vi.fn(async () => [{ role: "user" as const, content: "full history" }]);
  const dispatch = vi.fn();
  const legacy = mount({ history }, dispatch);
  await legacy.hook.load();
  expect(dispatch.mock.calls.at(-1)?.[0].items[0].text).toBe("full history");
  expect(legacy.hook.paging.hasMore.value).toBe(false);
  legacy.unmount();
  const historyPage = vi.fn().mockRejectedValue(new HttpError(501, "unsupported", { code: "history.pagination_unsupported" }));
  const optional = mount({ history, historyPage }, dispatch);
  await optional.hook.load();
  expect(history).toHaveBeenCalledTimes(2);
  expect(optional.hook.paging.failed.value).toBe(false);
});

it("discards an old binding's late page", async () => {
  const old = deferred<HistoryPage>();
  const dispatch = vi.fn();
  const first = { history: vi.fn(async () => []), historyPage: vi.fn(() => old.promise) };
  const second = { history: vi.fn(async () => []), historyPage: vi.fn(async () => page("new", "")) };
  const { hook, bind } = mount(first, dispatch);
  const pending = hook.load();
  await bind(second);
  await hook.load();
  old.resolve(page("old"));
  await pending;
  await nextTick();
  expect(dispatch).toHaveBeenCalledTimes(1);
  expect(hook.paging.snapshot.value).toBe("new");
});

it("reloads a changed snapshot without appending stale rows", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockResolvedValueOnce(page())
    .mockRejectedValueOnce(new HttpError(409, "changed", { code: "history.snapshot_changed" })).mockResolvedValueOnce(page("new", "")) };
  const dispatch = vi.fn();
  const { hook } = mount(port, dispatch);
  await hook.load();
  await hook.paging.more();
  await nextTick();
  expect(hook.paging.snapshot.value).toBe("new");
  expect(dispatch.mock.calls.map(([event]) => event.kind)).toEqual(["__restore", "__restore"]);
});

it("searches beyond the loaded tail and stops requesting pages when the query is cleared", async () => {
  const older = deferred<HistoryPage>();
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockResolvedValueOnce({ ...page(), start: 3, end: 4, total: 4 }).mockReturnValueOnce(older.promise) };
  const dispatch = vi.fn();
  const query = ref("");
  const { hook } = mountSearch(port, dispatch, query);
  await hook.load();
  query.value = "needle";
  await nextTick();
  expect(port.historyPage).toHaveBeenCalledTimes(2);
  query.value = "";
  await nextTick();
  older.resolve({ ...page("one", "yet-older"), start: 1, end: 3, total: 4, messages: [{ role: "user", content: "needle", msgIndex: 1 }] });
  await waitFor(() => expect(dispatch.mock.calls.at(-1)?.[0].items[0].text).toBe("needle"));
  expect(port.historyPage).toHaveBeenCalledTimes(2);
});

it("refuses malformed successful responses without falling back to full history", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockResolvedValue(null) };
  const { hook } = mount(port, vi.fn());
  await expect(hook.load()).rejects.toThrow("invalid history page");
  expect(port.history).not.toHaveBeenCalled();
});

it("continues searching when fast page responses batch busy state transitions", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn()
    .mockResolvedValueOnce({ ...page("one", "middle"), start: 4, end: 6, total: 6 })
    .mockResolvedValueOnce({ ...page("one", "first"), start: 2, end: 4, total: 6 })
    .mockResolvedValueOnce({ ...page("one", ""), start: 0, end: 2, total: 6 }) };
  const dispatch = vi.fn();
  const { hook } = mountSearch(port, dispatch, ref("needle"));
  await hook.load();
  await waitFor(() => expect(hook.paging.hasMore.value).toBe(false));
  expect(port.historyPage.mock.calls).toEqual([[], ["middle"], ["first"]]);
  expect(dispatch.mock.calls.map(([event]) => event.kind)).toEqual(["__restore", "__history_prepend", "__history_prepend"]);
});

it("searches the next page when a committed revision is the only changed read state", async () => {
  const more = vi.fn(async () => {});
  const paging: HistoryPaging = { snapshot: ref("one"), revision: ref(1), hasMore: ref(true), busy: ref(false), failed: ref(false), earlier: ref(0), more, reload: vi.fn(async () => {}) };
  render(defineComponent({
    setup() {
      useHistorySearch(paging, true, "needle");
      return () => h("div");
    },
  }));
  expect(more).toHaveBeenCalledTimes(1);
  paging.revision.value = 2;
  await nextTick();
  expect(more).toHaveBeenCalledTimes(2);
});

it("does not treat a missing runtime as an unsupported paging route", async () => {
  const port = { history: vi.fn(async () => []), historyPage: vi.fn().mockRejectedValue(new HttpError(404, "gone", { code: "runtime.not_found" })) };
  const { hook } = mount(port, vi.fn());
  await expect(hook.load()).rejects.toBeInstanceOf(HttpError);
  expect(port.history).not.toHaveBeenCalled();
});