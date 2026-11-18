import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { HubPort, TreeWorkspace } from "../port/hub";

const KEY = "reasonix:pinned-sessions";

function legacy(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function keep(paths: Set<string>) {
  try {
    if (paths.size === 0) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, JSON.stringify([...paths]));
  } catch {
    // the pins come back from the kernel on the next load
  }
}

// The kernel owns pins because its automatic archiving has to honour them, so
// what is drawn is what the tree reports. localStorage only carries pins made
// before the kernel kept them: each is sent once, when its conversation is
// first listed, and forgotten. A toggle shows at once and is dropped as soon
// as the tree agrees; a refused one is withdrawn and reported.
export function usePinnedSessions(hub: MaybeRefOrGetter<HubPort>, tree: MaybeRefOrGetter<TreeWorkspace[]>, treeRead: MaybeRefOrGetter<boolean>, onFailure: (e: unknown) => void = () => {}) {
  const shown = ref<Map<string, boolean>>(new Map());
  const old: { current: Set<string> | null } = { current: null };
  let opened = false;

  const kernel = computed(() => {
    const out = new Set<string>();
    for (const ws of toValue(tree)) for (const s of ws.sessions) if (s.pinned) out.add(s.path);
    return out;
  });

  const pinned = computed(() => {
    const out = new Set(kernel.value);
    for (const [path, on] of shown.value) {
      if (on) out.add(path);
      else out.delete(path);
    }
    return out;
  });

  watch(kernel, () => {
    const cur = shown.value;
    const next = new Map([...cur].filter(([path, on]) => kernel.value.has(path) !== on));
    if (next.size !== cur.size) shown.value = next;
  }, { immediate: true });

  const show = (path: string, on: boolean | null) => {
    const next = new Map(shown.value);
    if (on === null) next.delete(path);
    else next.set(path, on);
    shown.value = next;
  };

  const toggle = (path: string) => {
    const on = !pinned.value.has(path);
    show(path, on);
    return toValue(hub).pinSession(path, on).catch((e) => {
      show(path, null);
      onFailure(e);
    });
  };

  const unpin = (path: string) => show(path, false);

  watch([() => toValue(hub), () => toValue(tree), () => toValue(treeRead), kernel], () => {
    if (!toValue(treeRead)) return;
    old.current ??= legacy();
    const listed = new Set<string>();
    for (const ws of toValue(tree)) for (const s of ws.sessions) listed.add(s.path);
    const move = [...old.current].filter((path) => listed.has(path));
    const send = move.filter((path) => !kernel.value.has(path));
    if (opened && send.length === 0) {
      if (move.length > 0) {
        for (const path of move) old.current.delete(path);
        keep(old.current);
      }
      return;
    }
    toValue(hub)
      .syncPins(send)
      .then(() => {
        opened = true;
        for (const path of move) old.current?.delete(path);
        if (old.current) keep(old.current);
        for (const path of send) show(path, true);
      })
      .catch(() => {});
  }, { immediate: true });

  return [pinned, toggle, unpin] as const;
}