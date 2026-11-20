// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/vue";
import userEvent from "@testing-library/user-event";
import { defineComponent, h, nextTick, ref } from "vue";
import { useModalFocus } from "./useModalFocus";

afterEach(cleanup);

const Dialog = defineComponent({
  name: "Dialog",
  props: { name: { type: String, required: true } },
  setup(props, { slots }) {
    const root = ref<HTMLDivElement | null>(null);
    useModalFocus(root);
    return () => h("div", { ref: root, role: "dialog", "aria-modal": "true", "aria-label": props.name, tabindex: -1 }, [
      h("button", `${props.name} first`),
      h("button", `${props.name} last`),
      slots.default?.(),
    ]);
  },
});

const Page = defineComponent({
  name: "Page",
  setup() {
    const open = ref(false);
    const inner = ref(false);
    return () => [
      h("button", { onClick: () => { open.value = true; } }, "open"),
      h("button", "behind"),
      open.value
        ? h(Dialog, { name: "outer" }, () => [
          h("button", { onClick: () => { inner.value = true; } }, "open inner"),
          h("button", { onClick: () => { open.value = false; } }, "close outer"),
          inner.value ? h(Dialog, { name: "inner" }) : null,
        ])
        : null,
    ];
  },
});

it("wraps Tab and Shift+Tab inside the dialog instead of reaching the page behind it", async () => {
  render(Page);
  await userEvent.click(screen.getByText("open"));
  await nextTick();
  const first = screen.getByText("outer first");
  const closeOuter = screen.getByText("close outer");
  first.focus();
  await userEvent.tab({ shift: true });
  expect(document.activeElement).toBe(closeOuter);
  await userEvent.tab();
  expect(document.activeElement).toBe(first);
  for (let i = 0; i < 12; i++) {
    await userEvent.tab();
    expect(document.querySelector('[aria-label="outer"]')!.contains(document.activeElement)).toBe(true);
  }
});

it("gives focus back to the control that opened it", async () => {
  render(Page);
  const opener = screen.getByText("open");
  await userEvent.click(opener);
  await nextTick();
  await userEvent.click(screen.getByText("close outer"));
  await nextTick();
  expect(document.activeElement).toBe(opener);
});

it("lets only the innermost of two open dialogs hold Tab", async () => {
  render(Page);
  await userEvent.click(screen.getByText("open"));
  await nextTick();
  await userEvent.click(screen.getByText("open inner"));
  await nextTick();
  screen.getByText("inner last").focus();
  await userEvent.tab();
  expect(document.activeElement).toBe(screen.getByText("inner first"));
  await userEvent.tab({ shift: true });
  expect(document.activeElement).toBe(screen.getByText("inner last"));
});

it.each([
  { visible: false, expected: { preventScroll: true, focusVisible: false } },
  { visible: true, expected: { preventScroll: true } },
])("gives focus back with the ring only if the opener had one (visible=$visible)", async ({ visible, expected }) => {
  const user = userEvent.setup();
  render(Page);
  const opener = screen.getByRole("button", { name: "open" });
  const realMatches = opener.matches.bind(opener);
  vi.spyOn(opener, "matches").mockImplementation(((selector: string) => (selector === ":focus-visible" ? visible : realMatches(selector))) as typeof opener.matches);
  const focus = vi.spyOn(opener, "focus");
  await user.click(opener);
  await nextTick();
  await user.click(screen.getByRole("button", { name: "close outer" }));
  await nextTick();
  expect(focus).toHaveBeenLastCalledWith(expected);
});