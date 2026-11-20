export function openCards(root: ParentNode = document) {
  for (const d of root.querySelectorAll<HTMLDetailsElement>("details.tool-disclosure:not([open])")) {
    d.open = true;
    d.dispatchEvent(new Event("toggle"));
  }
}