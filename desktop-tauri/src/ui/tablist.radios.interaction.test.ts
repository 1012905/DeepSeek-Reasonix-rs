// @vitest-environment jsdom
import { defineComponent, nextTick, ref } from "vue";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/vue";
import { arrowRadios } from "./tablist";

afterEach(cleanup);

// Vue TL's fireEvent resolves to undefined, so the preventDefault answer is read
// from dispatchEvent itself — the same boolean the DOM event carries.
const keyDown = (el: Element, init: KeyboardEventInit) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));

it("wraps enabled radios with focus and click, leaving locked or unrelated keys alone", async () => {
  const selected = vi.fn();
  const Radios = defineComponent({
    props: { locked: { type: Boolean, default: false } },
    setup() {
      const at = ref("a");
      const pick = (id: string) => { at.value = id; selected(id); };
      return { at, pick, arrowRadios };
    },
    template: `<div role="radiogroup" @keydown="arrowRadios">
      <button v-for="id in ['a', 'b', 'c', 'd']" :key="id" type="button" role="radio" :aria-checked="at === id"
        :tabindex="at === id ? 0 : -1" :disabled="locked || id === 'b'"
        @click="pick(id)">{{ id }}</button>
    </div>
    <button type="button">outside</button>`,
  });
  const view = render(Radios);
  const radio = (name: string) => screen.getByRole<HTMLButtonElement>("radio", { name });
  radio("a").focus();
  for (const [key, name] of [["ArrowRight", "c"], ["ArrowDown", "d"], ["ArrowDown", "a"], ["ArrowLeft", "d"], ["ArrowUp", "c"], ["ArrowUp", "a"]]) {
    expect(keyDown(document.activeElement!, { key })).toBe(false);
    await nextTick();
    expect(document.activeElement).toBe(radio(name));
    expect(radio(name).getAttribute("aria-checked")).toBe("true");
    expect(radio(name).tabIndex).toBe(0);
    expect(selected).toHaveBeenLastCalledWith(name);
  }
  expect(selected.mock.calls.map(([name]) => name)).toEqual(["c", "d", "a", "d", "c", "a"]);
  for (const modifiers of [{ altKey: true }, { ctrlKey: true }, { metaKey: true }]) {
    expect(keyDown(radio("a"), { key: "ArrowRight", ...modifiers })).toBe(true);
  }
  for (const key of ["Enter", " ", "Home", "End", "Escape", "x"]) expect(keyDown(radio("a"), { key })).toBe(true);
  expect(selected).toHaveBeenCalledTimes(6);
  expect(document.activeElement).toBe(radio("a"));
  screen.getByRole("button", { name: "outside" }).focus();
  expect(keyDown(screen.getByRole("radiogroup"), { key: "ArrowRight" })).toBe(true);
  expect(selected).toHaveBeenCalledTimes(6);
  await view.rerender({ locked: true });
  expect(keyDown(radio("a"), { key: "ArrowRight" })).toBe(true);
  expect(selected).toHaveBeenCalledTimes(6);
  expect(radio("a").getAttribute("aria-checked")).toBe("true");
});