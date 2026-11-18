// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/vue";
import "../testkit";
import { MockPort } from "../../port/mock";
import type { AgentPort } from "../../port/port";
import { boot, t } from "../../i18n";
import DocumentPreview from "./DocumentPreview.vue";

beforeEach(() => boot());
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const preview = (port: AgentPort, path: string) =>
  render(DocumentPreview, { props: { port, path, shown: true, refresh: "", view: {} } });

describe("the workbench document preview", () => {
  it("renders an HTML file in the static sandboxed viewer", async () => {
    const port = new MockPort() as unknown as AgentPort;
    const { container } = preview(port, "page.html");
    await waitFor(() => expect(container.querySelector("iframe.document-html")).toBeTruthy());
    expect(screen.getByRole("button", { name: t("common.re_freeze") })).toBeTruthy();
  });

  it("says a preview is not available for a kind it cannot render", async () => {
    const port = new MockPort() as unknown as AgentPort;
    preview(port, "archive.zip");
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain(t("chat.preview_is_not_available_for")));
  });

  it("surfaces a failed read rather than an empty pane", async () => {
    const port = new MockPort() as unknown as AgentPort;
    vi.spyOn(port, "workspacePreviewInfo").mockRejectedValue(new Error("disk gone"));
    preview(port, "page.html");
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("disk gone"));
  });

  it("offers the system application when a file is over the preview limit", async () => {
    const port = new MockPort() as unknown as AgentPort;
    vi.spyOn(port, "workspacePreviewInfo").mockResolvedValue({
      ref: { path: "big.html" }, kind: "html", size: 9_000_000, limit: 2 << 20, version: "v", readOnly: true,
    });
    preview(port, "big.html");
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain(t("workbench.file_too_large_to_preview")));
  });
});