// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/vue";
import type { AgentPort, ContextBreakdown, JobEntry } from "../../port/port";
import AgentsPanel from "./Agents.vue";
import JobsPanel from "./Jobs.vue";
import CachePanel from "./Cache.vue";
import ContextPanel from "./Context.vue";
import RuntimePanel from "./Runtime.vue";
import ExtensionsPanel from "./Extensions.vue";
import CostPanel from "./Cost.vue";
import Metrics from "../Metrics.vue";

// The rail's contract is "a panel with nothing to say draws nothing", so most of
// what is worth guarding here is absence: a block that appears with a zero in it
// is the nine-panel wall this replaced.

afterEach(cleanup);

const noop = () => {};

const status = (over: Record<string, unknown> = {}) => ({
  label: "m", running: false, plan: false, preset: "", toolApprovalMode: "ask",
  autoApproveTools: false, bypass: false, goal: "", goalStatus: "", cwd: "",
  used: 0, window: 0, cacheHit: 0, cacheMiss: 0, ...over,
});

const port = (over: Record<string, unknown> = {}) => ({
  status: async () => status(),
  subagents: async () => [],
  processes: async () => [],
  extensionSurfaces: async () => ({ surfaces: [] }),
  subscribe: () => () => {},
  balance: async () => null,
  setContextWindow: async () => breakdown({}),
  ...over,
}) as unknown as AgentPort;

const breakdown = (over: Partial<ContextBreakdown>): ContextBreakdown => ({
  used: 42000, window: 131072, system: 9000, tools: 12000, user: 4000, reply: 9000, output: 8000,
  compact_at: 160000, boundary: "", capacity_at: 0, ...over,
});

describe("the context panel", () => {
  it("draws the bar against the window and names every class it drew", async () => {
    const { container } = render(ContextPanel, { props: { port: port(), context: breakdown({}), onError: noop } });
    await waitFor(() => expect(container.querySelector(".ctxbar")).not.toBeNull());
    expect(container.textContent).toContain("131k");
    // Five slices, five labels: a slice with no legend row is a colour nobody
    // can look up.
    expect(container.querySelectorAll(".ctxlg .r")).toHaveLength(5);
  });

  // A zero window switches automatic compaction off, so vanishing said neither
  // that nor the number; the panel has to keep speaking.
  it("still states the usage when nobody declared a window", () => {
    const { container } = render(ContextPanel, { props: { port: port(), context: breakdown({ window: 0 }), onError: noop } });
    expect(container.textContent).toContain("42k");
    expect(container.querySelector(".ctxbar")).toBeNull();
  });

  it("draws nothing at all without a reading", () => {
    const { container } = render(ContextPanel, { props: { port: port(), context: null, onError: noop } });
    expect(container.textContent?.trim() ?? "").toBe("");
  });

  // A fold point past the window is how maintenance is retired: there is no
  // deadline left to draw, so the capacity row is the whole gauge.
  it("drops the fold row when the fold point is past the window", async () => {
    const { container } = render(ContextPanel, { props: { port: port(), context: breakdown({ compact_at: 200000 }), onError: noop } });
    await waitFor(() => expect(container.querySelector(".ctxbar")).not.toBeNull());
    expect(container.querySelector(".ctxcap")).toBeNull();
  });
});

describe("the cache panel", () => {
  it("says nothing until a request has been made", () => {
    const { container } = render(CachePanel, { props: { port: port(), onError: noop } });
    expect(container.textContent?.trim() ?? "").toBe("");
  });

  it("states the rate it was given, as a ratio of what was split", async () => {
    render(CachePanel, { props: { port: port({ status: async () => status({ cacheHit: 60, cacheMiss: 40 }) }), onError: noop } });
    await waitFor(() => expect(screen.getByText("60.0%")).toBeTruthy());
  });

  // Prompt the endpoint never split into cached and uncached is out of the
  // denominator (hit + miss - unknown), or it would pull a rate down that
  // nobody measured. cacheRate owns that arithmetic; this checks the panel
  // uses it rather than dividing hit by hit + miss.
  it("leaves unsplit prompt out of the denominator", async () => {
    const split = { requests: 1, promptTokens: 1000, completionTokens: 0, cacheHit: 90, cacheMiss: 10, workMs: 0, sources: [] };
    const { container } = render(CachePanel, {
      props: { port: port({ status: async () => status({ cacheHit: 90, cacheMiss: 10, sessionUsage: { ...split, cacheUnknown: 90 } }) }), onError: noop },
    });
    await waitFor(() => expect(container.querySelector(".bar")).not.toBeNull());
    expect(container.textContent).toContain("100.0%");
  });
});

describe("the jobs panel", () => {
  const job = (over: Partial<JobEntry> = {}): JobEntry =>
    ({ id: "bash-1", kind: "bash", label: "npm test", status: "running", startedAt: Date.now() - 5000, ...over });

  it("says nothing with no background jobs", () => {
    const { container } = render(JobsPanel, { props: { port: port(), onError: noop } });
    expect(container.textContent?.trim() ?? "").toBe("");
  });

  // One reading per row: a running row that printed both its elapsed time and
  // its status said the same second twice.
  it("gives a running job exactly one time reading", async () => {
    const { container } = render(JobsPanel, { props: { port: port({ status: async () => status({ jobs: [job()], endedJobs: [] }) }), onError: noop } });
    await waitFor(() => expect(container.querySelector(".job")).not.toBeNull());
    const row = container.querySelector(".job")!;
    expect(row.querySelectorAll(".rt")).toHaveLength(1);
    expect(row.querySelector(".rt")?.textContent).toMatch(/5\s*(s|秒)/);
  });

  it("offers stopped jobs by their outcome rather than a duration nobody measured", async () => {
    const ended = job({ id: "bash-2", status: "failed", endedAt: undefined });
    const { container } = render(JobsPanel, { props: { port: port({ status: async () => status({ jobs: [], endedJobs: [ended] }) }), onError: noop } });
    await waitFor(() => expect(container.querySelector(".job")).not.toBeNull());
    expect(container.querySelector(".job")?.textContent).not.toMatch(/\d+\s*s\b/);
  });

  // Cancel is a request the kernel can refuse (job.not_running), and a button
  // that fired on the first press would be the refusal nobody asked for.
  it("arms a stop before it asks for one", async () => {
    const { container } = render(JobsPanel, { props: { port: port({ status: async () => status({ jobs: [job()], endedJobs: [] }) }), onError: noop } });
    await waitFor(() => expect(container.querySelector(".job-stop")).not.toBeNull());
    expect(container.querySelector(".job-confirm")).toBeNull();
  });
});

describe("the panels that speak only when they have something", () => {
  // Cost is not in this list on purpose: a session that has not been billed
  // still draws its zero, labelled with why it is not a bill. Only the three
  // that are cumulative counters fall silent at zero.
  const silent: [string, typeof AgentsPanel][] = [
    ["agents", AgentsPanel],
    ["runtime", RuntimePanel],
    ["extensions", ExtensionsPanel],
  ];

  it.each(silent)("%s draws nothing for an idle session", (_name, Comp) => {
    const { container } = render(Comp, { props: { port: port(), onError: noop } });
    expect(container.textContent?.replace(/\s+/g, "")).toBe("");
  });

  it("agents lists a run the kernel reports", async () => {
    const port0 = port({ subagents: async () => [{ ref: "r1", name: "探路", kind: "explore", model: "deepseek/v4", state: "running", startedAt: Date.now() - 3000, updatedAt: Date.now() }] });
    render(AgentsPanel, { props: { port: port0, onError: noop } });
    await waitFor(() => expect(screen.getByText("探路")).toBeTruthy());
  });
});

describe("the cost panel", () => {
  // A session that has not been billed still draws its zero, as it always has —
  // and the zero is labelled with why it is not a bill, rather than reading as
  // a session that cost nothing.
  it("draws the zero with the reason it is not a bill", async () => {
    const { container } = render(CostPanel, { props: { port: port(), onError: noop } });
    await waitFor(() => expect(container.textContent).toContain("0"));
    expect(container.querySelector(".mtop")).not.toBeNull();
  });
});

describe("the container", () => {
  // The anchors exist whether or not the panel inside drew: `at` has to be able
  // to land on one, and a wrapper that vanished with its panel could not.
  it("anchors every panel it owns", () => {
    const { container } = render(Metrics, { props: { port: port(), context: breakdown({}), mcp: [], surfaces: 0, onError: noop } });
    for (const id of ["context", "runtime", "agents", "jobs", "cost", "cache", "extensions", "mcp"]) {
      expect(container.querySelector(`[data-panel="${id}"]`), id).not.toBeNull();
    }
  });

  // A wrapper with a box of its own would take every .block out of the rail's
  // column and with it the gap the other blocks have.
  it("keeps its wrappers out of the layout", () => {
    const { container } = render(Metrics, { props: { port: port(), context: breakdown({}), mcp: [], surfaces: 0, onError: noop } });
    for (const el of container.querySelectorAll("[data-panel]")) {
      expect((el as HTMLElement).style.display).toBe("contents");
    }
  });

  // The files panel reads the pane's walk handed down as props, not a second
  // derivation: with changes it draws a row per file git still reports.
  it("draws the worktree changes the pane hands it", () => {
    const { container } = render(Metrics, { props: {
      port: port(), context: breakdown({}), mcp: [], surfaces: 0, onError: noop,
      changes: [{ path: "src/a.ts", added: 3, removed: 1, edits: 2 }],
      tree: { repo: true, changes: [{ path: "src/a.ts", status: "M" }] },
      yolo: false,
    } });
    const files = container.querySelector('[data-b="files"]');
    expect(files).not.toBeNull();
    expect(files!.textContent).toContain("a.ts");
    expect(files!.textContent).toContain("+3");
  });

  it("draws no files block when the pane reports no changes", () => {
    const { container } = render(Metrics, { props: {
      port: port(), context: breakdown({}), mcp: [], surfaces: 0, onError: noop,
      changes: [], tree: null, yolo: false,
    } });
    expect(container.querySelector('[data-b="files"]')).toBeNull();
  });
});
