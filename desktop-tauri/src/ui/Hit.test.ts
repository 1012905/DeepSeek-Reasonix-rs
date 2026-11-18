// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import Hit from "./Hit.vue";

afterEach(cleanup);

it("marks the first match whatever its case and keeps the surrounding text", () => {
  const { container } = render(Hit, { props: { text: "Fix the Login bug", needle: "login" } });
  expect(container.querySelector("mark.hit")?.textContent).toBe("Login");
  expect(container.textContent).toBe("Fix the Login bug");
});

it("renders plain text without a needle or a match", () => {
  expect(render(Hit, { props: { text: "abc", needle: "" } }).container.querySelector("mark")).toBeNull();
  cleanup();
  expect(render(Hit, { props: { text: "abc", needle: "z" } }).container.querySelector("mark")).toBeNull();
});