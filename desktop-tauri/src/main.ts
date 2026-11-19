import { boot as bootLang, t } from "./i18n";
import { reason } from "./i18n/kernel";
import { track as trackWidth } from "./ui/viewport";
import "./styles/legacy.css";
import "./design/design.css";
import { createApp } from "vue";
import App from "./ui/App.vue";
import { SseHub } from "./port/hub";
import { HttpError } from "./port/http_error";
import type { HubPort } from "./port/hub";
import { host } from "./port/host";
import { play } from "./boot/intro";
import { settled } from "./boot/gate";

// The dev proxy only exists when REASONIX_SERVE was set at vite start; probing
// /runtimes decides which port to boot on, so neither mode needs a build flag.
// Without the proxy vite answers it with the SPA shell at 200, so the content
// type — not res.ok — is what says a kernel is really there.
async function pick(): Promise<HubPort> {
  try {
    // The hub's own list rather than a pane's /status: it answers with every
    // pane closed, which is when a reload has to find the kernel again.
    const res = await fetch("/runtimes", { credentials: "same-origin" });
    if (res.ok && (res.headers.get("content-type") ?? "").includes("json")) return new SseHub();
    // A kernel that answered and refused: a phone that is not paired, or no
    // longer is. Its code says which, and "cannot reach" would say neither.
    if (res.status === 401) {
      const body = (await res.json().catch(() => null)) as { code?: string; error?: string } | null;
      if (body?.code) throw new HttpError(res.status, body.error ?? "", body);
    }
  } catch (e) {
    if (e instanceof HttpError) throw e;
    // no serve reachable
  }
  // A shipped build is served by the kernel it talks to. Falling back to the
  // fixture there would put a scripted session on screen as if it had happened,
  // so only a dev build is allowed to; the import stays dynamic to keep the
  // fixture out of the production bundle.
  if (!import.meta.env.DEV) throw new Error(t("app.cannot_reach_the_kernel_runtimes"));
  const { MockHub } = await import("./port/mock_hub");
  return new MockHub();
}

// A shell that hides the native title bar lets its lights float over the page,
// so the chrome has to reserve their corner and be draggable itself. Which of
// those is true is the shell's answer, not something the page infers from a
// platform or from which globals it can see.
void host()
  .describe()
  .then(({ shell, platform, titleBar }) => {
    const root = document.documentElement.dataset;
    root.shell = shell;
    if (platform) root.platform = platform;
    if (titleBar) root.titlebar = "app";
  });

// macOS hides its traffic lights outright on an inactive window rather than
// greying them, so the corner they were reserved is simply empty. The wordmark
// takes the slot back while they are gone; the slot itself never resizes.
const focus = () => {
  document.documentElement.dataset.focused = document.hasFocus() ? "yes" : "no";
};
addEventListener("focus", focus);
addEventListener("blur", focus);
focus();

// Before the first paint: a language that arrives after one would show the
// interface in one language and then swap it.
bootLang();
trackWidth();

// The boot screen is markup in index.html, so it is on screen before this
// bundle is parsed; from here it plays the intro while the kernel is asked.
const intro = play(document.getElementById("boot"));
intro.progress(1);
// A shell that never settles — no pane to ask on — must not keep the window
// behind the wordmark forever.
const SETTLE_CAP_MS = 6000;

// The window is handed over only when both are true: the intro has put the
// wordmark down, and the app has something of its own to show. Whichever is
// later decides; until then the wordmark holds under its glint.
function arrive(shown: Promise<unknown>) {
  const root = document.documentElement;
  const cap = new Promise((done) => setTimeout(done, SETTLE_CAP_MS));
  void Promise.all([intro.held, Promise.race([shown, cap])])
    .then(() => new Promise(requestAnimationFrame))
    .then(() => {
      // Arms the app's own entrance under the opening hole, then lets go of
      // it: a .app that remounts later must not replay the window opening.
      root.dataset.boot = "in";
      setTimeout(() => delete root.dataset.boot, 2200);
      return intro.leave();
    })
    .then(() => document.getElementById("boot")?.remove());
}

const escape = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);

pick().then(
  (hub) => {
    try { localStorage.removeItem("rx-flow-w"); } catch { /* storage is a convenience */ }
    const root = document.getElementById("root");
    if (root) createApp(App, { hub }).mount(root);
    intro.progress(2);
    void settled.then(() => intro.progress(3));
    arrive(settled);
  },
  (e: unknown) => {
    const root = document.getElementById("root");
    if (root) root.innerHTML = `<div class="app" data-run="idle"><div class="errbar" role="alert"><span>${escape(reason(e))}</span></div></div>`;
    // The failure is the one thing that must not stay behind the boot screen.
    arrive(Promise.resolve());
  },
);