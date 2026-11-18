// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/vue";
import userEvent from "@testing-library/user-event";
import Button from "./Button.vue";

afterEach(cleanup);

it("renders one shared button class with its variant and size", () => {
  render(Button, { slots: { default: "Save" } });
  render(Button, { props: { variant: "primary", size: "md" }, slots: { default: "Send" } });
  render(Button, { props: { variant: "ghost", iconOnly: true }, attrs: { "aria-label": "More" }, slots: { default: "…" } });
  const save = screen.getByRole("button", { name: "Save" });
  expect(save.className).toBe("kbtn");
  expect(save.dataset.variant).toBe("secondary");
  expect(save.dataset.size).toBe("sm");
  expect(save.getAttribute("type")).toBe("button");
  const send = screen.getByRole("button", { name: "Send" });
  expect([send.dataset.variant, send.dataset.size]).toEqual(["primary", "md"]);
  expect(screen.getByRole("button", { name: "More" }).hasAttribute("data-icon-only")).toBe(true);
});

it("passes clicks, disabled state and extra attributes through", async () => {
  const click = vi.fn();
  render(Button, { attrs: { "data-action": "x.go", class: "wide", onClick: click }, slots: { default: "Go" } });
  render(Button, { attrs: { disabled: true, onClick: click }, slots: { default: "Off" } });
  const go = screen.getByRole("button", { name: "Go" });
  expect(go.className).toBe("kbtn wide");
  expect(go.dataset.action).toBe("x.go");
  await userEvent.click(go);
  await userEvent.click(screen.getByRole("button", { name: "Off" }));
  expect(click).toHaveBeenCalledTimes(1);
});