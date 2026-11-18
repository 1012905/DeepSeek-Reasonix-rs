import { computed, onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter, type Ref } from "vue";
import { HttpError, type AgentPort } from "../port/port";
import type { HistoryPage } from "../port/session";
import { fromHistory, type SessionEvent } from "../state/session";

export interface HistoryPaging {
  snapshot: Ref<string>;
  revision: Ref<number>;
  hasMore: Ref<boolean>;
  busy: Ref<boolean>;
  failed: Ref<boolean>;
  /** How many raw messages sit before the loaded window. The rail draws only
   *  what is mounted, so without this a paged conversation looks shorter than
   *  it is until someone scrolls up. */
  earlier: Ref<number>;
  more: () => Promise<void>;
  reload: () => Promise<void>;
}

export function useHistorySearch(paging: HistoryPaging, open: MaybeRefOrGetter<boolean>, query: MaybeRefOrGetter<string>) {
  watch([() => toValue(open), () => toValue(query), paging.revision, paging.hasMore, paging.busy, paging.failed], () => {
    if (toValue(open) && toValue(query).trim() && paging.hasMore.value && !paging.busy.value && !paging.failed.value) void paging.more().catch(() => {});
  }, { immediate: true });
}

const projection = (page: HistoryPage) => fromHistory(Array.isArray(page.messages) ? page.messages : [], {
  snapshot: page.snapshot, toolContext: Array.isArray(page.toolContext) ? page.toolContext : [], turnStartedAt: page.turnStartedAt,
});

function checkedPage(page: HistoryPage): HistoryPage {
  if (!page || !Array.isArray(page.messages) || typeof page.snapshot !== "string" ||
      !Number.isSafeInteger(page.start) || !Number.isSafeInteger(page.end) || !Number.isSafeInteger(page.total) ||
      page.start < 0 || page.end < page.start || page.total < page.end ||
      page.hasMoreBefore !== (page.start > 0) || page.hasMoreBefore && !page.nextCursor) throw new Error("invalid history page");
  return page;
}

function unsupported(error: unknown): boolean {
  if (!(error instanceof HttpError)) return false;
  const code = error.reason?.code;
  return code === "history.pagination_unsupported" || !code && [404, 405, 501].includes(error.status);
}

export function useHistoryPages(port: MaybeRefOrGetter<Pick<AgentPort, "history" | "historyPage">>, dispatch: (event: SessionEvent) => void) {
  const binding = () => ({ epoch: 0, cursor: "", snapshot: "", before: 0, total: 0, paging: !!toValue(port).historyPage, more: null as Promise<void> | null });
  let owner = binding();
  let current = owner;
  const state = ref({ snapshot: "", revision: 0, hasMore: false, busy: false, failed: false, earlier: 0 });
  watch(() => toValue(port), () => {
    owner.epoch++;
    owner = binding();
    current = owner;
  });
  onScopeDispose(() => { owner.epoch++; });

  const load = async (applies: () => boolean = () => true) => {
    const agent = toValue(port);
    const mine = owner;
    const epoch = ++mine.epoch;
    const valid = () => current === mine && epoch === mine.epoch && applies();
    mine.cursor = "";
    mine.more = null;
    state.value = { ...state.value, hasMore: false, busy: true, failed: false };
    try {
      let page: HistoryPage | undefined;
      if (mine.paging && agent.historyPage) {
        try { page = checkedPage(await agent.historyPage()); }
        catch (error) {
          if (!unsupported(error)) throw error;
          mine.paging = false;
        }
      }
      const messages = page ? null : await agent.history();
      if (!valid()) return;
      const restored = page ? projection(page) : fromHistory(messages ?? []);
      mine.cursor = page?.nextCursor ?? "";
      mine.snapshot = page?.snapshot ?? "";
      mine.before = page?.start ?? 0;
      mine.total = page?.total ?? 0;
      dispatch({ kind: "__restore", ...restored });
      state.value = { snapshot: mine.snapshot, revision: state.value.revision + 1, hasMore: !!mine.cursor, busy: false, failed: false, earlier: mine.before };
    } catch (error) {
      if (valid()) state.value = { ...state.value, busy: false, failed: true };
      throw error;
    }
  };

  const more = (): Promise<void> => {
    const agent = toValue(port);
    const mine = owner;
    if (mine.more) return mine.more;
    if (!mine.cursor || !agent.historyPage) return Promise.resolve();
    const epoch = mine.epoch;
    const cursor = mine.cursor;
    const valid = () => current === mine && epoch === mine.epoch;
    state.value = { ...state.value, busy: true, failed: false };
    const pending: Promise<void> = agent.historyPage(cursor).then((page) => {
      if (!valid()) return;
      checkedPage(page);
      if (page.snapshot !== mine.snapshot) return load();
      if (page.end !== mine.before || page.total !== mine.total || page.start >= page.end) throw new Error("history page did not advance");
      mine.cursor = page.nextCursor ?? "";
      mine.before = page.start;
      mine.more = null;
      dispatch({ kind: "__history_prepend", ...projection(page) });
      state.value = { ...state.value, revision: state.value.revision + 1, hasMore: !!mine.cursor, busy: false, failed: false, earlier: mine.before };
    }).catch(async (error) => {
      if (!valid()) return;
      if (error instanceof HttpError && error.reason?.code === "history.snapshot_changed") { await load(); return; }
      state.value = { ...state.value, busy: false, failed: true };
      throw error;
    }).finally(() => { if (mine.more === pending) mine.more = null; });
    mine.more = pending;
    return pending;
  };

  const reload = () => load();
  const paging: HistoryPaging = {
    snapshot: computed(() => state.value.snapshot),
    revision: computed(() => state.value.revision),
    hasMore: computed(() => state.value.hasMore),
    busy: computed(() => state.value.busy),
    failed: computed(() => state.value.failed),
    earlier: computed(() => state.value.earlier),
    more,
    reload,
  };
  return { load, paging };
}