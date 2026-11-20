import { computed, toValue, type ComputedRef, type MaybeRefOrGetter } from "vue";
import type { Item } from "../state/session";

// How many settled cards a mounting unit gathers before it closes at the next
// turn. Small enough that scrolling mounts a block within a frame, large enough
// that a long session holds a few hundred blocks rather than tens of thousands.
const BLOCK = 48;

// A streamed delta rewrites only the last card, so the settled head is cut into
// blocks that keep their array identity across frames and each block can memo.
// Anything that edits a card mid-list bumps the revision, so re-cutting on
// revision or cut alone walks the blocks once per turn rather than per frame.
export function useBlocks(
  items: MaybeRefOrGetter<Item[]>,
  cut: MaybeRefOrGetter<number>,
  revision: MaybeRefOrGetter<number>,
): ComputedRef<Item[][]> {
  let held: { at: number; cut: number; blocks: Item[][] } = { at: -1, cut: -1, blocks: [] };
  return computed(() => {
    const its = toValue(items);
    const c = toValue(cut);
    const r = toValue(revision);
    if (held.at === r && held.cut === c) return held.blocks;

    const prev = held.blocks;
    const previousByFirst = new Map(prev.map((block) => [block[0]?.id, block]));
    const blocks: Item[][] = [];
    for (let at = 0, end = 0; at < c; at = end) {
      const candidate = previousByFirst.get(its[at].id);
      const keep = candidate && at + candidate.length <= c && candidate.every((item, index) => item === its[at + index]);
      const boundary = keep && (at + candidate.length === c || opensTurn(its[at + candidate.length]));
      if (keep && boundary && (candidate !== prev[prev.length - 1] || at + candidate.length === c || candidate.length >= BLOCK)) {
        blocks.push(candidate);
        end = at + candidate.length;
        continue;
      }
      end = turnEnd(its, at, c);
      while (end < c && end - at < BLOCK && !previousByFirst.has(its[end].id)) end = turnEnd(its, end, c);
      const old = candidate;
      const same = old?.length === end - at && old.every((item, index) => item === its[at + index]);
      blocks.push(same && old ? old : its.slice(at, end));
    }
    held = { at: r, cut: c, blocks };
    return blocks;
  });
}

// A late history read can put older rows before an existing form in its block.
// The surviving rows own that mounting identity, not the new first row.
export function useBlockKeys(blocks: MaybeRefOrGetter<Item[][]>): ComputedRef<string[]> {
  let held: { blocks: Item[][] | null; keys: string[]; owners: Map<string, string>; next: number } = { blocks: null, keys: [], owners: new Map(), next: 0 };
  return computed(() => {
    const bs = toValue(blocks);
    if (held.blocks === bs) return held.keys;
    const used = new Set<string>();
    const next = new Map<string, string>();
    const keys = bs.map((block) => {
      let retained: string | undefined;
      for (const item of block) {
        const prior = held.owners.get(item.id);
        if (prior !== undefined && !used.has(prior)) { retained = prior; break; }
      }
      const key = retained ?? `block:${++held.next}`;
      used.add(key);
      for (const item of block) next.set(item.id, key);
      return key;
    });
    held = { ...held, blocks: bs, keys, owners: next };
    return keys;
  });
}

// A block ends only where a turn does: transcriptRows hangs work under its
// sentence within one block, so a turn cut in two leaves half its work with no
// sentence to sit under.
function turnEnd(items: Item[], from: number, cut: number): number {
  let end = from + 1;
  while (end < cut && !opensTurn(items[end])) end++;
  return end;
}

// A queued line has not happened yet, and a steer was said into a turn already
// running; neither is a turn of its own.
export const opensTurn = (it: Item) => it.t === "user" && !it.pending && !it.steer;