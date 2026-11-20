import { computed, onMounted, onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { host } from "../port/host";
import type { AgentPort } from "../port/port";
import type { RuntimeView, TreeWorkspace } from "../port/hub";
import { rememberUnreadSince, sessionKey } from "../state/newAnswers";

type Trees = Record<string, TreeWorkspace[] | null>;

const keyOf = sessionKey;

// The kernel's answer with the rows the person has just opened drawn as seen.
// Same reference back when nothing is cleared, so a memoised sidebar stays put.
export function seen(tree: TreeWorkspace[], host: string, cleared: Set<string>): TreeWorkspace[] {
  if (cleared.size === 0) return tree;
  let changed = false;
  const next = tree.map((ws) => {
    if (!ws.sessions.some((s) => s.unread && cleared.has(keyOf(host, s.path)))) return ws;
    changed = true;
    return { ...ws, sessions: ws.sessions.map((s) => (s.unread && cleared.has(keyOf(host, s.path)) ? { ...s, unread: false } : s)) };
  });
  return changed ? next : tree;
}

export function isUnread(rt: RuntimeView, tree: TreeWorkspace[], remote: Trees): boolean {
  if (!rt.sessionPath) return false;
  const book = rt.host ? remote[rt.host] ?? [] : tree;
  return book.some((ws) => ws.sessions.some((s) => s.path === rt.sessionPath && !!s.unread));
}

function unreadSince(rt: RuntimeView, tree: TreeWorkspace[], remote: Trees): number | undefined {
  if (!rt.sessionPath) return undefined;
  const book = rt.host ? remote[rt.host] ?? [] : tree;
  for (const ws of book) for (const s of ws.sessions) if (s.path === rt.sessionPath && s.unread) return s.viewedAt;
  return undefined;
}

interface Inputs {
  runtimes: MaybeRefOrGetter<RuntimeView[]>;
  ports: MaybeRefOrGetter<Map<string, AgentPort>>;
  active: MaybeRefOrGetter<string>;
  tree: MaybeRefOrGetter<TreeWorkspace[]>;
  remote: MaybeRefOrGetter<Trees>;
  reload: () => Promise<unknown>;
}

// Viewing is the window's decision and the kernel never infers it. The pane in
// front is seen when it becomes the one in front, when a turn ends in it, and
// whenever it is unread while the window has focus, whichever of the pane, its
// list or its path arrived last. A refused call puts the mark back.
export function useViewed({ runtimes, ports, active, tree, remote, reload }: Inputs) {
  const cleared = ref<Set<string>>(new Set());
  const now = computed(() => ({ runtimes: toValue(runtimes), ports: toValue(ports), tree: toValue(tree), remote: toValue(remote), reload, active: toValue(active) }));
  const inflight = new Set<string>();

  const again = new Set<string>();
  const held = new Set<string>();

  const mark = (id: string, owed: boolean) => {
    const cur = now.value;
    const rt = cur.runtimes.find((r) => r.id === id);
    const port = cur.ports.get(id);
    if (!rt?.sessionPath || !port) return;
    const key = keyOf(rt.host, rt.sessionPath);
    if (inflight.has(key)) {
      if (owed) again.add(key);
      return;
    }
    if (!owed && !isUnread(rt, cur.tree, cur.remote)) return;
    const since = unreadSince(rt, cur.tree, cur.remote);
    if (!owed && since) rememberUnreadSince(key, since);
    inflight.add(key);
    const set = (on: boolean) => {
      if (cleared.value.has(key) === on) return;
      const next = new Set(cleared.value);
      if (on) next.add(key);
      else next.delete(key);
      cleared.value = next;
    };
    set(true);
    port
      .markSessionViewed()
      .then(() => now.value.reload().then(() => true, () => false))
      .catch(() => null)
      .then((read) => {
        inflight.delete(key);
        if (read === false) held.add(key);
        else set(false);
        const owes = again.delete(key);
        const rt = now.value.runtimes.find((r) => r.id === id);
        if (owes && rt?.sessionPath && keyOf(rt.host, rt.sessionPath) === key) mark(id, true);
      });
  };

  const view = (id: string) => mark(id, false);
  const turnDone = (id: string) => {
    if (id === now.value.active && document.hasFocus()) mark(id, true);
  };

  const focus = () => { if (document.hasFocus()) view(now.value.active); };
  onMounted(() => window.addEventListener("focus", focus));
  onScopeDispose(() => window.removeEventListener("focus", focus));

  let front = toValue(active);
  watch([() => toValue(active), () => toValue(ports)], () => {
    const at = toValue(active);
    const book = toValue(ports);
    if (front === at || (at && !book.has(at))) return;
    front = at;
    if (at) view(at);
  }, { immediate: true });

  const frontUnread = computed(() => {
    const rt = toValue(runtimes).find((r) => r.id === toValue(active));
    return !!rt && isUnread(rt, toValue(tree), toValue(remote));
  });
  watch([() => toValue(active), frontUnread, () => toValue(ports)], () => {
    const at = toValue(active);
    if (at && frontUnread.value && toValue(ports).has(at) && document.hasFocus()) view(at);
  }, { immediate: true });

  watch([() => toValue(tree), () => toValue(remote)], () => {
    if (held.size === 0) return;
    const drop = new Set(held);
    held.clear();
    const next = new Set([...cleared.value].filter((k) => !drop.has(k)));
    if (next.size !== cleared.value.size) cleared.value = next;
  }, { immediate: true });

  const local = computed(() => seen(toValue(tree), "", cleared.value));
  const far = computed(() => {
    const out: Trees = {};
    for (const [host, book] of Object.entries(toValue(remote))) out[host] = book ? seen(book, host, cleared.value) : book;
    return out;
  });

  return { view, turnDone, tree: local, remote: far };
}

export function useUnreadBadge(tree: MaybeRefOrGetter<TreeWorkspace[]>, remote: MaybeRefOrGetter<Trees>) {
  const unread = computed(() => [toValue(tree), ...Object.values(toValue(remote))].some((book) => book?.some((ws) => ws.sessions.some((s) => s.unread && !s.archived))));
  watch(unread, (on) => { void host().setUnreadBadge(on).catch(() => {}); }, { immediate: true });
}