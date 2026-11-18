import { cloneVNode, isVNode, type VNode, type VNodeChild } from "vue";

type Row = VNodeChild[];

function join(head: Row[], tail: Row[]): Row[] {
  return [...head.slice(0, -1), [...head[head.length - 1], ...tail[0]], ...tail.slice(1)];
}

function rowsOf(node: VNodeChild): Row[] {
  if (node == null || typeof node === "boolean") return [[]];
  if (typeof node === "number") return [[String(node)]];
  if (typeof node === "string") return node.split("\n").map((piece) => (piece === "" ? [] : [piece]));
  if (Array.isArray(node)) return node.reduce<Row[]>((all, child) => join(all, rowsOf(child)), [[]]);
  if (!isVNode(node)) return [[]];
  const element = node as VNode;
  const inner = rowsOf(element.children as VNodeChild);
  if (inner.length === 1) return [[element]];
  return inner.map((row, i) => {
    if (row.length === 0) return [];
    const cloned = cloneVNode(element, { key: `${String(element.key ?? "n")}-${i}` });
    cloned.children = row;
    return [cloned];
  });
}

/** The rows of a highlighted tree, one per source line. An element that spans a
 *  newline is repeated in every row it touches, so each row stands on its own. */
export function splitLines(tree: VNodeChild): VNodeChild[][] {
  return rowsOf(tree);
}