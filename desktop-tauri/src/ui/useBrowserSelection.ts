import { computed, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { AgentPort, BrowserTab } from "../port/port";
import { failedPageID, observedPageURLs, pageReadAfterSettlement, recheckPageTabs, reconcilePageFollow, unseenPageOpenings } from "./browser_open";

/** A newer choice supersedes user-opened pages across both their response and
 *  tab-list notification. Unrelated agent pages remain eligible to follow. */
export function useBrowserSelection(port: MaybeRefOrGetter<AgentPort>, select: (key: string) => void, tabs: MaybeRefOrGetter<BrowserTab[]>) {
  const latestTabs = computed(() => toValue(tabs));
  let revision = 0;
  let ignored = new Set<Promise<BrowserTab>>();
  const following = { issued: 0, selected: 0 };
  watch(() => toValue(port), (_agent, _was, onCleanup) => {
    ignored = new Set();
    onCleanup(() => { revision++; });
  }, { immediate: true });
  const supersede = () => {
    revision++;
    ignored = new Set(unseenPageOpenings(toValue(port)));
  };
  const current = () => {
    const at = revision;
    return () => at === revision;
  };
  const pick = (key: string) => {
    supersede();
    select(key);
  };
  const follow = (key: string) => {
    const agent = toValue(port);
    const choiceCurrent = current(), order = ++following.issued;
    const valid = () => choiceCurrent() && order >= following.selected;
    const apply = () => { if (valid()) { following.selected = order; select(key); } };
    // Snapshot before this tab-list batch retires its observed requests. A
    // later agent navigation of the same target is a different operation.
    const pending = unseenPageOpenings(agent, { startedOnly: true }).filter((page) => ignored.has(page));
    if (!pending.length) { apply(); return; }
    const tab = latestTabs.value.find((tab) => `browser:${tab.target}` === key);
    // A later navigation can precede the open response. Capture only locations
    // observed before this event, not ones that arrive while its promise waits.
    const prior = pending.map((page) => tab ? observedPageURLs(agent, page, tab.id) : new Set<string>());
    const fresh = pending.map((page) => pageReadAfterSettlement(agent, latestTabs.value, page));
    void Promise.allSettled(pending).then(async (results) => {
      if (!valid()) return;
      const changed = (opened: BrowserTab) => opened.url !== tab?.url || (opened.active === false && tab?.active === true);
      const owns = (result: PromiseSettledResult<BrowserTab>, index: number, confirmed = false) => result.status === "fulfilled"
        ? `browser:${result.value.target}` === key && (!changed(result.value) || (!confirmed && !fresh[index] && (result.value.url === tab?.url || !prior[index].has(result.value.url))))
        : !!tab && failedPageID(result.reason) === tab.id;
      if (!results.some((result, index) => owns(result, index))) { apply(); return; }
      const ambiguous = results.some((result, index) => owns(result, index) && result.status === "fulfilled" && changed(result.value));
      if (!tab || !ambiguous) return;
      // A final-URL read can be canceled before the response arrives. Verify
      // this candidate afresh instead of guessing which navigation it was.
      const next = await recheckPageTabs(agent, pending).catch(() => null);
      if (!next) return;
      const confirmed = results.map((result, index) => result.status === "fulfilled" && reconcilePageFollow(agent, pending[index], next, tab, result.value));
      if (!valid() || latestTabs.value.find((item) => item.target === tab.target)?.url !== tab.url) return;
      if (next.some((item) => item.target === tab.target && item.url === tab.url) && !results.some((result, index) => owns(result, index, confirmed[index]))) apply();
    });
  };
  return { pick, follow, supersede, current };
}