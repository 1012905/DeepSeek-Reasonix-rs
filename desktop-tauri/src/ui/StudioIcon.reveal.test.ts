// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/vue";
import StudioIcon from "./StudioIcon.vue";

describe("the reveal-in-file-manager icon", () => {
  it("reads as an opened folder, not as a share or external-link arrow", () => {
    const { container } = render(StudioIcon, { props: { name: "reveal" } });
    expect(container.innerHTML).toContain("lucide-folder-open");
    expect(container.innerHTML).not.toContain("lucide-external-link");
  });
});