import { loadEnv } from "vite";
import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import { pdfAssets } from "./tools/pdf-assets.ts";

interface ProxyEvents {
  on(event: "proxyReq", cb: (req: { setHeader(k: string, v: string): void }) => void): void;
}

// Prefixes: vite matches a proxy key against the head of the path, so "/mcp"
// also carries /mcp/reconnect. A route missing here answers with the SPA shell
// instead of JSON, which is indistinguishable from a broken endpoint.
const ROUTES = [
  // The hub's own surface, plus /rt — every pane's requests carry that prefix,
  // so without it a second session's traffic answers with the SPA shell.
  "/runtimes", "/tree", "/rt", "/remotes", "/side", "/terminals",
  "/appearance",
  "/agent-presets", "/agent-composition",
  "/events", "/history", "/status", "/balance", "/submit", "/cancel", "/approve", "/answer",
  "/adjudications", "/execution-graph", "/jobs", "/tool-calls", "/subagents", "/processes",
  "/plan", "/plan-decision", "/goal", "/resume", "/models", "/tool-approval-mode", "/workspace-trust", "/preset",
  "/model", "/default-model", "/default-mode", "/effort", "/new", "/sessions", "/session/tail", "/delete-session", "/provider-setup",
  "/tools", "/present-files",
  "/inbox", "/trajectory", "/mcp", "/skills", "/complete", "/prompt", "/workspace", "/capability-scope",
  "/providers", "/decision-models", "/roles", "/account", "/backups", "/hooks", "/memory", "/network", "/shell", "/todos",
  "/changes", "/commit", "/attachments", "/drop", "/checkpoints", "/branches", "/compact", "/compaction", "/rewind", "/branch",
  "/extensions", "/themes", "/plugins", "/market", "/surfaces", "/feedback",
  "/fork", "/summarize", "/forget", "/bypass", "/auto-approve-tools",
  "/permissions", "/sandbox", "/progress-watch", "/ask-timeout", "/session-ref-settings", "/settings-changed", "/changed-files-settings", "/schedules", "/feedback-level-notices", "/observability", "/fold-work-settings", "/turn-narration-settings", "/display-currency", "/auto-archive", "/persona", "/context", "/storage", "/tray", "/browser", "/browser-host", "/asks", "/update", "/receipt-settings", "/subagent-claims", "/write-lease", "/peers",
  "/mods", "/git",
  "/host", "/notifications", "/share", "/pair", "/device", "/remote",
  "/web-access-settings", "/slash", "/workspaces", "/welcome", "/usage", "/config", "/studio",
];

// REASONIX_SERVE points at a running `reasonix serve`; without it the app boots
// on MockPort so the UI can be developed with no Go process at all.
export default defineConfig(({ mode }) => {
  const serve = loadEnv(mode, ".", "REASONIX").REASONIX_SERVE;
  return {
    base: "./",
    plugins: [vue(), pdfAssets()],
    clearScreen: false,
    envPrefix: ["VITE_", "TAURI_"],
    // Stylesheets are stubbed for tests by default, `?raw` included, so a
    // guard that reads app.css to check a rule gets an empty string and passes
    // on nothing. Processing it is what makes that guard able to fail.
    //
    // Threads rather than forks: a worker per file either way, but a thread is
    // cheaper to start, which is most of what a suite of small files pays.
    //
    // At most ten workers: every first lazy import goes through the one vite
    // transform queue, and past that a cold import waits longer than a test's
    // own budget, so files that pass alone in 0.4s fail in a full run.
    test: {
      // Disk-reading tests resolve import.meta.url as a file URL only under the
      // node transform; DOM tests opt into jsdom per file with a docblock.
      environment: "node",
      css: true,
      setupFiles: ["./src/testsetup.ts"],
      testTimeout: 60_000,
      hookTimeout: 60_000,
      pool: "threads",
      maxWorkers: Math.min(navigator.hardwareConcurrency, 10),
    },
    server: {
      port: 5280,
      strictPort: true,
      // The dev server is cross-origin to serve, which its CSRF guard rejects.
      // Rewriting Origin makes the hop look same-origin, matching production
      // where the UI really is served from the kernel.
      proxy: serve
        ? Object.fromEntries(
            ROUTES.map((r) => [
              r,
              {
                target: serve,
                changeOrigin: true,
                configure: (proxy: unknown) => {
                  (proxy as ProxyEvents).on("proxyReq", (req) => req.setHeader("origin", serve));
                },
              },
            ]),
          )
        : undefined,
    },
    build: {
      target: "chrome110",
      outDir: "dist",
      emptyOutDir: true,
    },
  };
});