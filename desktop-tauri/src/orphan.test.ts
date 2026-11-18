import { describe, expect, it } from "vitest";

// A component nobody renders still typechecks, still lints, and still passes
// every test that reads it: the file is right and no window ever shows it. The
// React tree this frontend replaced carried a guard for exactly that. The Vue
// port carried the file over and not the tree — its glob asked for *.ts and
// *.tsx, then kept only the keys ending in .tsx, of which there are none here.
// It read nothing, asserted nothing, and went green over 321 components it had
// never looked at, which is the failure it exists to catch.
//
// The walk is the bundler's own: literal import and export statements, the
// import() of a chunk, a worker's new URL(..., import.meta.url), and
// import.meta.glob itself. The glob is the one edge a reader of import
// statements cannot see, and this tree mounts twenty settings sections through
// exactly one of them — miss it and every section page reads as an orphan.
//
// Vite reads the sources; the frontend carries no Node types, so a check that
// needed them would only typecheck by accident.
const SOURCES = import.meta.glob(["./**/*.{ts,vue}", "!./**/*.test.*"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

// Entry points are reached by the bundler and by no import at all. Declared,
// never inferred: nothing in a filename says whether a page loads it.
const ENTRIES = ["src/main.ts", "src/testsetup.ts"];

/** One spelling for every file, rooted at the project rather than at whichever
 *  directory the glob happened to resolve it from. */
const root = (key: string): string => "src/" + key.replace(/^\.\//, "");

const FILES = new Set(Object.keys(SOURCES).map(root));

function resolve(from: string, spec: string): string {
  const stack = from.split("/").slice(0, -1);
  for (const part of spec.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") stack.pop();
    else stack.push(part);
  }
  return stack.join("/");
}

/** A path is written without its extension and a directory import lands on its
 *  index, so a file counts as reached under any of those spellings. */
const spellingsOf = (path: string): string[] => [
  path,
  path + ".ts",
  path + ".vue",
  path + "/index.ts",
  path + "/index.vue",
];

/** The three spellings of a glob this tree uses: * stops at a separator, **
 *  crosses one (and may stand for no directory at all), {a,b} picks. */
function globOf(pattern: string): RegExp {
  let out = "^";
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i];
    if (ch === "*" && pattern[i + 1] === "*") {
      i++;
      if (pattern[i + 1] === "/") {
        i++;
        out += "(?:.*/)?";
      } else out += ".*";
    } else if (ch === "*") out += "[^/]*";
    else if (ch === "?") out += "[^/]";
    else if (ch === "{") {
      const close = pattern.indexOf("}", i);
      out += "(?:" + pattern.slice(i + 1, close).split(",").join("|") + ")";
      i = close;
    } else if ("\\^$.|+()[]".includes(ch)) out += "\\" + ch;
    else out += ch;
  }
  return new RegExp(out + "$");
}

// A quoted relative path is what an import statement and a dynamic import both
// spell; the from arm also catches a re-export, which reaches a file just as
// surely and is how half of the state layer here is wired.
const IMPORT = /(?:\bfrom\s*|\bimport\s*\(?\s*)(["'])([^"'\n]+)\1/g;
const WORKER = /\bnew\s+URL\s*\(\s*(["'])([^"'\n]+)\1\s*,\s*import\.meta\.url/g;
const GLOB = /import\.meta\.glob\s*\(([\s\S]*?)\)/g;

function reachedFrom(from: string, body: string): string[] {
  const out = new Set<string>();
  const literal = (spec: string) => {
    if (spec.startsWith(".")) out.add(resolve(from, spec));
  };
  for (const m of body.matchAll(IMPORT)) literal(m[2]);
  for (const m of body.matchAll(WORKER)) literal(m[2]);
  for (const m of body.matchAll(GLOB)) {
    for (const s of m[1].matchAll(/(["'])([^"'\n]+)\1/g)) {
      // A negated pattern names what a glob leaves out, never what it imports.
      if (s[2].startsWith("!")) continue;
      const path = resolve(from, s[2]);
      if (!path.includes("*")) {
        out.add(path);
        continue;
      }
      const match = globOf(path);
      for (const file of FILES) if (match.test(file)) out.add(file);
    }
  }
  return [...out].flatMap(spellingsOf).filter((file) => FILES.has(file));
}

const EDGES = new Map<string, string[]>(
  Object.entries(SOURCES).map(([key, body]) => [root(key), reachedFrom(root(key), body)]),
);

/** Breadth-first from the entries, so a file counts only when a path of imports
 *  leads to it from something a browser or the test setup really loads. */
const REACHED = (() => {
  const seen = new Set(ENTRIES.filter((entry) => FILES.has(entry)));
  const queue = [...seen];
  while (queue.length) {
    for (const next of EDGES.get(queue.pop()!) ?? []) {
      if (seen.has(next)) continue;
      seen.add(next);
      queue.push(next);
    }
  }
  return seen;
})();

const ORPHANS = [...FILES].filter((file) => !REACHED.has(file)).sort();

// The debt the Vue port carries: files that landed, typecheck, and that no path
// of imports from main.ts or testsetup.ts reaches. Recorded rather than deleted
// because most of it is a column waiting for Pane.vue to hold it — and recorded
// one line per file rather than as a count, so that wiring one file while
// orphaning another in the same change cannot cancel out.
//
// This is a register of debt, never a place to park a new file. The first test
// fails on any orphan this list does not name, so it cannot grow; the second
// fails on any entry the tree reaches or no longer carries, so it only shrinks.
// Neither may be widened to land a change.
const KNOWN_ORPHANS = new Map<string, string>([
  // The action census. Its reader is a vitest guard, not a module the window loads.
  ["src/actions.ts", "only actions_document.ts, actions_feedback.ts +3 import it, none reached — The census oracle is independent of production literals so disagreements fail"],
  ["src/actions_document.ts", "only actions.ts imports it, itself unreached — exports DOCUMENT_ACTIONS"],
  ["src/actions_feedback.ts", "only actions.ts imports it, itself unreached — exports FEEDBACK_ACTIONS"],
  ["src/actions_market.ts", "only actions.ts imports it, itself unreached — The community market: reads are views, and the one write is the install"],
  ["src/actions_schedule.ts", "only actions.ts imports it, itself unreached — exports SCHEDULE_ACTIONS"],
  ["src/actions_workbench.ts", "only actions.ts imports it, itself unreached — exports WORKBENCH_ACTIONS"],
  // A probe driven from outside the bundle: shell_shim stands in for EventSource, graph_observer folds the frames.
  ["src/e2e/graph_observer.ts", "nothing imports it — Run by benchmarks/runtime-resume against a live kernel"],
  ["src/e2e/shell_shim.ts", "only e2e/graph_observer.ts imports it, itself unreached — The window the client expects, installed before anything imports it"],
  // A reducer or reader the Vue panes have not taken over; its own test is what holds it.
  ["src/state/execution.ts", "only e2e/graph_observer.ts imports it, itself unreached — Where this model stands relative to its authority"],
  ["src/state/graph.ts", "only state/execution.ts, ui/glayout.ts import it, none reached — The run graph, folded the way internal/contract/agentgraph folds it"],
  ["src/state/paneread.ts", "nothing imports it — Reads the pane list so that a failed read is never mistaken for an empty one"],
  ["src/state/rewind_notice.ts", "nothing imports it — exports dismissRewindNotice, useRewindNoticeDismissed"],
  // Landed with the Vue port and not yet imported by App.vue or Pane.vue.
  ["src/ui/AuthorLine.vue", "functionality inlined into settings/sections/versions.vue:59-62 (.comm-author, same key/data-action); redundant per D13"],
  ["src/ui/ChangedFilesPill.vue", "nothing imports it — The workspace's differing files as git counts them, in one place above the"],
  ["src/ui/CompositionNewContext.ts", "nothing imports it — exports CompositionNewContext"],
  ["src/ui/InstalledLocation.vue", "nothing imports it — The market hands over an applied action"],
  ["src/ui/ListingProgress.vue", "nothing imports it — the \"reading history summaries\" note above a workspace tree, with its retry"],
  ["src/ui/MemoryProactiveRow.vue", "nothing imports it — functionality inlined into settings/parts/data-memory.vue (.lrow/.seg); redundant per D13"],
  ["src/ui/NewAnswerLine.vue", "nothing imports it — the \"new answers start here\" separator, and the anchor it exports"],
  ["src/ui/PresetActions.vue", "nothing imports it — the expandable per-preset actions (persona, etc.) in the composition picker"],
  ["src/ui/ProcessDeck.vue", "nothing imports it — What this session has running now, one row per process"],
  ["src/ui/SessionBatch.vue", "nothing imports it — The selected rows that are still on screen"],
  ["src/ui/TariffChip.vue", "nothing imports it — How long until the flip, in the coarsest unit still true, never rounding a"],
  ["src/ui/WorkFold.vue", "nothing imports it — One turn's intermediate work under a single line"],
  ["src/ui/WorkbenchLauncher.vue", "nothing imports it — the \"open in the workbench\" launcher, taking changes, focus, onFiles, onBrowser"],
  ["src/ui/WsMark.vue", "nothing imports it — the folder chevron a workspace row draws, open/tri-state driving it"],
  ["src/ui/blocks.ts", "only ui/document/deliveries.ts imports it, itself unreached — How many settled cards a mounting unit gathers before it closes at the next"],
  ["src/ui/chunk_text.ts", "nothing imports it — How each block a selection touches must stay in the DOM"],
  ["src/ui/drawer.ts", "nothing imports it — Under the scene fold the rail is a drawer laid over the conversation rather"],
  ["src/ui/extpick.ts", "nothing imports it — Present only on the extensions page, where a row is chosen into the detail"],
  ["src/ui/feedbacktestkit.ts", "nothing imports it — The list row naming a report"],
  ["src/ui/foldaway.ts", "nothing imports it — Both side columns share one rule"],
  ["src/ui/glayout.ts", "nothing imports it — Node boxes are a fixed size, so a lane's geometry follows from its ranks with"],
  ["src/ui/gnode.ts", "nothing imports it — The calls this transcript holds, by the id a node names"],
  ["src/ui/goalusage.ts", "nothing imports it — What the kernel writes today"],
  ["src/ui/jumpreach.ts", "nothing imports it — Tops are positions in the scroller's content"],
  ["src/ui/listsearch.ts", "nothing imports it — exports listMatcher"],
  ["src/ui/machinebooks.ts", "nothing imports it — Machines whose book this window has read before"],
  ["src/ui/meterShare.ts", "nothing imports it — The share of max that value fills, as a whole percentage held between 0 and"],
  ["src/ui/navrail.ts", "nothing imports it — What the icon column does this render"],
  ["src/ui/newAnswerMark.ts", "nothing imports it — newAnswerAnchor names the item the \"new answers start here\" line sits above"],
  ["src/ui/openwith_label.ts", "nothing imports it — The board writes the editor's short name, which is what the menu has room for"],
  ["src/ui/pinorder.ts", "nothing imports it — A pinned conversation leads its project"],
  ["src/ui/policyview.ts", "nothing imports it — What the shelf says about how the next turn will run"],
  ["src/ui/protocolswitch.ts", "nothing imports it — What picking the other door on an account actually does"],
  ["src/ui/railfilter.ts", "nothing imports it — exports RailFilterContext"],
  ["src/ui/railgeom.ts", "nothing imports it — One drawn thing on the message rail"],
  ["src/ui/roots.ts", "nothing imports it — Where Studio's user inputs are found, once"],
  ["src/ui/settings-intents.ts", "nothing imports it — What people type when they look for a setting, and where it lives"],
  ["src/ui/sidebaropen.ts", "nothing imports it — Picking a conversation that is already on screen brings it back into view"],
  ["src/ui/speed.ts", "nothing imports it — Seconds until the latest measured round's first output"],
  ["src/ui/stable.ts", "only ui/usePaneCallbacks.ts imports it, itself unreached — A function whose identity never changes and which always runs the latest one"],
  ["src/ui/storageTally.ts", "nothing imports it — exports STORAGE_TALLY_TTL, STORAGE_TALLY_DELAY, storageTotal"],
  ["src/ui/testkit.ts", "nothing imports it — What jsdom does not implement and the components under test really call"],
  ["src/ui/trajectory-export.ts", "nothing imports it — exports spanText, serialiseTrajectory, trajectoryFileName"],
  ["src/ui/treewake.ts", "nothing imports it — exports watchTreeWake"],
  ["src/ui/treewindow.ts", "nothing imports it — The most rows of one folder the sidebar will hold"],
  ["src/ui/turnrows.ts", "nothing imports it — drawn reports whether a sentence has anything to show"],
  ["src/ui/useColumnOpening.ts", "nothing imports it — What the workbench column does when it is opened"],
  ["src/ui/useLiveGrace.ts", "nothing imports it — Which conversations the running list holds"],
  ["src/ui/useNewConversation.ts", "nothing imports it — exports useNewConversation"],
  ["src/ui/usePaneCallbacks.ts", "nothing imports it — What each memoized pane is handed"],
  ["src/ui/usePaneForSheets.ts", "nothing imports it — Settings and feedback are answered by a pane's kernel, so with none open"],
  ["src/ui/usePaneSessionRefs.ts", "nothing imports it — A past conversation the rail asked the focused pane's composer to hold as a"],
  ["src/ui/usePinnedSessions.ts", "nothing imports it — the pins come back from the kernel on the next load"],
  ["src/ui/useSessionMutation.ts", "nothing imports it — exports captureSessionFocus, useSessionMutation"],
  ["src/ui/useTabNames.ts", "browser-only tab rename; WorkbenchPanel.vue surfaces yield no browser kind (B13), so the rename UI would be unreachable"],
  ["src/ui/useWorkspaceDrag.ts", "nothing imports it — Reordering the remembered projects by dragging a row"],
  ["src/ui/usefind.ts", "nothing imports it — n moves on every step, so landing on the same occurrence twice is asked for"],
  ["src/ui/usefindask.ts", "nothing imports it — What the window asks of the focused pane's find bar"],
  ["src/ui/userewind.ts", "nothing imports it — Editing a message is a rewind composed with a send, not a capability of its"],
  ["src/ui/usesurfaceslots.ts", "nothing imports it — Where the user put each extension surface, and the split of the pane's views"],
  ["src/ui/cards/openCards.ts", "nothing imports it — exports openCards"],
  // The chart renderer's test-only fixtures; no production card imports them.
  ["src/ui/chart/fixtures.ts", "nothing imports it — exports SALES, withSpec, chartCall"],
  // Delivery grouping; the delivery-card chain the document viewers do not import.
  ["src/ui/document/deliveries.ts", "nothing imports it — exports deliveriesByReply"],
  // The shared primitive; no call site has chosen it over the ad-hoc markup yet.
  ["src/ui/kit/Button.vue", "nothing imports it — the shared Button primitive, taking variant, size and iconOnly"],
  // The two parts of the machines panel that need a machine's own conversation
  // tree: Remotes.vue draws the host book and opens panes, and the per-project
  // menu and per-conversation row belong to the column that lists them.
  ["src/ui/remote/RemoteProjectMenu.vue", "nothing imports it — One remote project's own menu"],
  ["src/ui/remote/RemoteSessionRow.vue", "nothing imports it — One conversation on another machine, as its own row"],
  // An extension surface, waiting for the pane's slotted view to mount one by name.
  ["src/ui/surface/SurfaceField.vue", "nothing imports it — one field of an extension surface; also the Radio.Card arrow-key walk"],
  ["src/ui/surface/media-context.ts", "nothing imports it — exports SurfaceMediaContext"],
  // The rich surface field, waiting for the surface host above it.
  ["src/ui/surface/rich/Workspace.vue", "nothing imports it — the rich (interactive) workspace surface; also FocusTrap's active state"],
] as [string, string][]);

describe("every source the tree carries is reached", () => {
  it("does not leave a file nothing imports", () => {
    const unrecorded = ORPHANS.filter((file) => !KNOWN_ORPHANS.has(file));
    expect(
      unrecorded,
      "nothing reaches this file from src/main.ts or src/testsetup.ts — import it where it belongs, or delete it; the recorded debt above is the only exception and it may not grow",
    ).toEqual([]);
  });

  it("keeps every recorded orphan an orphan", () => {
    const live = new Set(ORPHANS);
    const stale = [...KNOWN_ORPHANS.keys()]
      .filter((file) => !live.has(file))
      .map((file) => `${file} — ${FILES.has(file) ? "reached again" : "gone from the tree"}; drop it from the list`);
    expect(stale, "recorded as an orphan but no longer one — the debt shrank, so the entry has to go").toEqual([]);
  });

  it("reads the tree it is meant to be guarding", () => {
    expect(Object.keys(SOURCES).length, "the glob read almost nothing; the guard would pass over anything").toBeGreaterThan(250);
    expect([...FILES].filter((file) => file.endsWith(".vue")).length, "the .vue half of the tree stopped being read").toBeGreaterThanOrEqual(200);
    // The one glob edge the twenty settings pages hang from. Without it each of
    // them is an orphan at once, so this names the broken edge rather than
    // letting the pages be recorded as debt.
    const sections = [...FILES].filter((file) => file.startsWith("src/ui/settings/sections/") && file.endsWith(".vue"));
    expect(sections.length).toBeGreaterThanOrEqual(20);
    expect(sections.filter((file) => !REACHED.has(file)), "the settings sections are mounted only by import.meta.glob, and that edge is no longer read").toEqual([]);
  });
});
