// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from "@testing-library/vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MockPort } from "../../port/mock";
import WorkbenchPanel from "./WorkbenchPanel.vue";
import "../testkit";

afterEach(cleanup);

const settle = () => new Promise((r) => setTimeout(r, 0));

// The panel is what a pane mounts; these guards read what it does when it is
// really rendered, not what its own functions answer when called directly.
describe("the workbench panel", () => {
  it("draws the explorer from what the workspace answers, and opens a file from it", async () => {
    const port = new MockPort();
    const listing = vi.spyOn(port, "workspaceFiles").mockResolvedValue({ files: ["a.ts", "src/b.ts"], directories: ["src"] });
    const { container, findByText } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose: () => {}, visible: true },
    });
    await settle();
    expect(listing).toHaveBeenCalled();
    // Folders come first, and the tree is drawn under the explorer's own title.
    const rows = [...container.querySelectorAll(".workbench-tree-row")].map((row) => row.textContent?.trim());
    expect(rows).toEqual(["src", "a.ts"]);
    await fireEvent.click(await findByText("a.ts"));
    await settle();
    expect(container.querySelector('[data-key="file:a.ts"]')).not.toBeNull();
  });

  it("opens the changes the workspace lists, and reads one path's diff", async () => {
    const port = new MockPort();
    vi.spyOn(port, "changes").mockResolvedValue({ repo: true, changes: [{ path: "a.ts", status: "M", staged: false, unstaged: true }] });
    const diff = vi.spyOn(port, "changeDiff").mockResolvedValue({ path: "a.ts", diff: "@@ -1 +1 @@\n-a\n+b", truncated: false });
    const { container } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose: () => {}, visible: true },
    });
    await settle();
    await settle();
    expect(port.changes).toHaveBeenCalled();
    // The panel's explorer draws the change as a row to open.
    const row = container.querySelector('[data-action="workbench.diff"][data-target="a.ts"]');
    expect(row).not.toBeNull();
    await fireEvent.click(row!);
    await settle();
    await settle();
    expect(diff).toHaveBeenCalled();
  });

  it("says what went wrong rather than drawing an empty tree it cannot explain", async () => {
    const port = new MockPort();
    vi.spyOn(port, "workspaceFiles").mockRejectedValue(new Error("the folder is gone"));
    const { findByRole } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose: () => {}, visible: true },
    });
    expect(await findByRole("alert")).toHaveProperty("textContent", expect.stringContaining("the folder is gone"));
  });

  it("closes itself only when the panel's own close button is pressed", async () => {
    const port = new MockPort();
    const onClose = vi.fn();
    const { container } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose, visible: true },
    });
    await settle();
    expect(onClose).not.toHaveBeenCalled();
    await fireEvent.click(container.querySelector('[data-action="workbench.dismiss"]')!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("opens a previewable file in its document view, with a way back to the source", async () => {
    const port = new MockPort();
    vi.spyOn(port, "workspaceFiles").mockResolvedValue({ files: ["page.html"], directories: [] });
    const { container, findByText } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose: () => {}, visible: true },
    });
    await settle();
    await fireEvent.click(await findByText("page.html"));
    await settle();
    // A file with a document view lands in it, and the bar offers the source back.
    expect(container.querySelector('[data-action="workbench.mode"][data-value="document"]')).not.toBeNull();
    expect(container.querySelector(".document-preview")).not.toBeNull();
    await waitFor(() => expect(container.querySelector("iframe.document-html")).not.toBeNull());
  });

  it("takes a file link once and reports the file it put on screen", async () => {
    const port = new MockPort();
    vi.spyOn(port, "workspaceFiles").mockResolvedValue({ files: ["a.ts"], directories: [] });
    vi.spyOn(port, "workspaceFile").mockResolvedValue({ path: "a.ts", text: "x", truncated: false, binary: false } as never);
    const consumed = vi.fn();
    const onActiveFile = vi.fn();
    const { container } = render(WorkbenchPanel, {
      props: {
        port, root: "/w", onError: () => {}, onClose: () => {}, visible: true,
        fileRequest: { path: "a.ts", consumed }, onActiveFile,
      },
    });
    await waitFor(() => expect(container.querySelector('[data-key="file:a.ts"]')).not.toBeNull());
    expect(consumed).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onActiveFile).toHaveBeenCalledWith("a.ts"));
  });

  it("opens a background job as a workbench tab and reads its output", async () => {
    const port = new MockPort();
    const read = vi.spyOn(port, "jobOutput").mockResolvedValue({ text: "ok\n", status: "completed", bytes: 3, truncated: false });
    const { container } = render(WorkbenchPanel, {
      props: {
        port, root: "/w", onError: () => {}, onClose: () => {}, visible: true,
        agentRequest: { jobs: [{ id: "job-1", name: "npm test" }] },
      },
    });
    await waitFor(() => expect(container.querySelector('[data-key="job:job-1"]')).not.toBeNull());
    expect(read).toHaveBeenCalledWith("job-1");
    await waitFor(() => expect(container.querySelector(".jobrun")).not.toBeNull());
  });

  it("opens a sub-agent run as a workbench tab, reads it from the kernel, and closes it", async () => {
    const port = new MockPort();
    const run = { ref: "sa_1", name: "audit", kind: "task", model: "deepseek/pro", state: "completed" as const, startedAt: 1, updatedAt: 2 };
    const read = vi.spyOn(port, "subagentTranscript").mockResolvedValue({ run, messages: [], after: 0, next: 0, total: 0, live: false });
    const { container } = render(WorkbenchPanel, {
      props: { port, root: "/w", onError: () => {}, onClose: () => {}, visible: true, agentRequest: { runs: [{ ref: "sa_1", name: "audit" }] } },
    });
    await waitFor(() => expect(container.querySelector('[data-key="agent:sa_1"]')).not.toBeNull());
    expect(read).toHaveBeenCalledWith("sa_1", 0);
    expect(container.querySelector(".agentrun")).not.toBeNull();
    await fireEvent.click(container.querySelector('[data-key="agent:sa_1"] [data-action="workbench.close"]')!);
    await settle();
    expect(container.querySelector('[data-key="agent:sa_1"]')).toBeNull();
  });
});
