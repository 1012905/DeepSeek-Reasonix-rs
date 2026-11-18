import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import type { HubPort, TreeWorkspace } from "../port/hub";
import type { RemoteHost } from "../port/remote";

// Machines whose book this window has read before. Their sessions are what a
// person comes back for after a restart, so the window reads them again on
// start instead of waiting for a pane to connect.
const READ_KEY = "rx-remote-books";

function readBefore(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(READ_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function remember(host: string) {
  const all = readBefore();
  if (all.includes(host)) return;
  try {
    localStorage.setItem(READ_KEY, JSON.stringify([...all, host]));
  } catch {
    /* the book is still read on demand; only the start-up read is lost */
  }
}

/** Every machine's session book, the one way to re-read them, and a read of
 *  one machine that does not need a pane open on it.
 *
 *  A far book goes stale for the same reason this machine's does — a turn ends
 *  and the kernel names what it just wrote — so the window re-reads both on
 *  that one beat rather than leaving each half to remember.
 */
export function useMachineBooks(hub: HubPort, remotes: MaybeRefOrGetter<RemoteHost[] | null>) {
  const trees = ref<Record<string, TreeWorkspace[] | null>>({});

  // One machine, connected or not: the kernel takes a link for the read when
  // no pane holds one. A failure is the caller's to show.
  const read = async (host: string) => {
    const tree = await hub.remoteTree(host);
    trees.value = { ...trees.value, [host]: tree };
    if (tree) remember(host);
  };

  // Read through the live list: depending on it would re-create reload on
  // every poll of it, and with it whatever effect holds the caller.
  const reload = async () => {
    const live = (toValue(remotes) ?? []).filter((h) => h.status === "connected" || h.status === "degraded");
    for (const host of live) await read(host.name).catch(() => {});
  };

  // A machine that has just connected has a book nobody has read yet. Keyed by
  // the connected set, so a second one arriving does not re-read the first.
  const connected = computed(() => (toValue(remotes) ?? [])
    .filter((h) => h.status === "connected" || h.status === "degraded")
    .map((h) => h.name)
    .join(" "));
  watch(connected, (c) => {
    if (c) void reload();
  }, { immediate: true });

  // Once per window, the machines read before, among those still configured.
  let started = false;
  watch(() => toValue(remotes), (r) => {
    if (started || !r) return;
    started = true;
    const known = new Set(r.map((h) => h.name));
    for (const name of readBefore()) if (known.has(name)) void read(name).catch(() => {});
  }, { immediate: true });

  return { trees, reload, read };
}