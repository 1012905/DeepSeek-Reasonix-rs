<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { ShellChoice, TerminalInfo } from "../../port/terminal";

export type SizeChange = "larger" | "smaller" | "reset";

export interface TerminalLayerProps {
  port: AgentPort;
  /** Every terminal of the session, in tab order; each keeps its own screen. */
  terminals: TerminalInfo[];
  /** The one in front. Only it is read from the kernel; the rest hold their last frame. */
  active: string;
  visible: boolean;
  scheme: "light" | "dark";
  shells: ShellChoice[];
  onLoadShells: () => void;
  onShell: (kind: string) => void;
  /** Absent when nothing can take a selection: the float then offers copy only. */
  onQuote?: (text: string) => void;
  onOpenPage: (url: string) => void;
  onCommand: (id: string, line: string) => void;
  onRestart: (id: string) => Promise<void>;
  onClose: (id: string) => void;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { TerminalFrame, TerminalStream } from "../../port/terminal";
import StudioIcon from "../StudioIcon.vue";
import { monoFamily, terminalTheme } from "../terminal_theme";
import { noteSeen, seeNote, setTerminalSize, terminalSize, TERMINAL_SIZES } from "../terminal_prefs";
import TerminalHead from "./TerminalHead.vue";

const props = defineProps<TerminalLayerProps>();

const RECONNECT_MS = 1000;
const RECONNECTS = 5;

interface Screen {
  host: HTMLElement;
  term: Terminal;
  fit: FitAddon;
  watch: ResizeObserver | null;
  stream: TerminalStream | null;
  attempts: number;
  timer: number;
}

interface Float { x: number; y: number; text: string; url?: string }

const screens = new Map<string, Screen>();
// The command line typed since the last Enter, per terminal: a tab is named for
// what it is running while the shell is busy.
const typed = new Map<string, string>();
const rounds = ref<Record<string, number>>({});
const exits = ref<Record<string, number>>({});
const failures = ref<Record<string, string>>({});
const floats = ref<Record<string, Float | null>>({});
const note = ref(!noteSeen());
const size = ref(terminalSize());

const shownId = computed(() => (props.visible && props.active ? props.active : ""));

const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

const ended = (info: TerminalInfo): number | null =>
  info.id in exits.value ? exits.value[info.id] : info.running ? null : (info.exitCode ?? 0);

const floatOf = (id: string): Float | null => floats.value[id] ?? null;
const say = (id: string, next: Float | null) => { floats.value = { ...floats.value, [id]: next }; };

function fitScreen(screen: Screen) {
  // A container with no box has no grid to give xterm, and a fit taken then
  // resizes the shell to nonsense; only a laid-out screen is fitted.
  if (!screen.host.offsetParent) return;
  try {
    screen.fit.fit();
  } catch {
    /* not laid out yet; the resize observer asks again */
  }
}

// The kernel keeps what the shell wrote, so a stream that ends is reopened and
// the shell's own screen is replayed. One that keeps ending is given up on.
function readFrom(id: string, screen: Screen) {
  let live = true;
  const open = () => {
    screen.stream = props.port.streamTerminal(
      id,
      (frame: TerminalFrame) => {
        if (!live) return;
        if (frame.kind === "replay") {
          screen.term.reset();
          screen.attempts = 0;
          if (frame.data) screen.term.write(bytes(frame.data));
        } else if (frame.kind === "out" && frame.data) {
          screen.term.write(bytes(frame.data));
        } else if (frame.kind === "exit") {
          screen.attempts = RECONNECTS;
          exits.value = { ...exits.value, [id]: frame.code ?? 0 };
        }
      },
      () => {
        if (!live || screen.attempts >= RECONNECTS) return;
        screen.attempts += 1;
        screen.timer = window.setTimeout(() => { if (live) open(); }, RECONNECT_MS);
      },
    );
  };
  open();
  return () => {
    live = false;
    window.clearTimeout(screen.timer);
    screen.stream?.close();
    screen.stream = null;
  };
}

let stopReading: (() => void) | null = null;

function track(id: string, screen: Screen) {
  screen.term.onData((data) => {
    void props.port.terminalInput(id, data).catch(() => {});
    if (data.startsWith("\x1b")) return;
    for (const ch of data) {
      const line = typed.get(id) ?? "";
      if (ch === "\r") {
        typed.set(id, "");
        if (line.trim()) props.onCommand(id, line.trim());
      } else if (ch === "\x7f") typed.set(id, line.slice(0, -1));
      else if (ch >= " ") typed.set(id, line + ch);
    }
  });
  screen.term.onResize(({ cols, rows }) => void props.port.resizeTerminal(id, cols, rows).catch(() => {}));
  screen.term.onSelectionChange(() => {
    if (!screen.term.hasSelection() && floatOf(id)?.url) say(id, null);
  });
  screen.term.attachCustomKeyEventHandler((e) => {
    // Ctrl/Cmd+C copies the selection rather than sending ^C while one exists.
    if (e.type === "keydown" && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && screen.term.hasSelection()) {
      void navigator.clipboard.writeText(screen.term.getSelection());
      return false;
    }
    return true;
  });
}

function make(id: string, host: HTMLElement) {
  const term = new Terminal({
    fontFamily: monoFamily(host), fontSize: size.value, lineHeight: 1.55, cursorBlink: true, scrollback: 5000,
    theme: terminalTheme(host), allowProposedApi: false,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.loadAddon(new WebLinksAddon((event, url) => {
    const at = host.getBoundingClientRect();
    say(id, { x: event.clientX - at.left, y: event.clientY - at.top, text: url, url });
  }));
  term.open(host);
  const screen: Screen = { host, term, fit, watch: null, stream: null, attempts: 0, timer: 0 };
  screen.watch = typeof ResizeObserver === "function" ? new ResizeObserver(() => fitScreen(screen)) : null;
  screen.watch?.observe(host);
  track(id, screen);
  screens.set(id, screen);
  fitScreen(screen);
  void props.port.resizeTerminal(id, term.cols, term.rows).catch(() => {});
  return screen;
}

function drop(id: string) {
  const screen = screens.get(id);
  screens.delete(id);
  typed.delete(id);
  if (!screen) return;
  screen.watch?.disconnect();
  window.clearTimeout(screen.timer);
  screen.stream?.close();
  screen.term.dispose();
}

// Each screen's element, kept as Vue hands it over: a function ref runs again on
// every patch, so it only records, and sync does the one-time build.
const hosts = new Map<string, HTMLElement>();

function attach(id: string, el: Element | null) {
  if (el instanceof HTMLElement) hosts.set(id, el);
  else hosts.delete(id);
}

/** The screen is built the first time its tab is in front: xterm cannot be
 *  fitted while hidden, and one built at the wrong size repaints on the first
 *  resize. */
function sync() {
  stopReading?.();
  stopReading = null;
  const id = shownId.value;
  const host = id ? hosts.get(id) : undefined;
  if (!id || !host) return;
  const screen = screens.get(id) ?? make(id, host);
  fitScreen(screen);
  stopReading = readFrom(id, screen);
  screen.term.focus();
}

watch([() => props.terminals.map((x) => x.id).join("\n"), shownId, rounds], () => {
  for (const id of [...screens.keys()]) if (!props.terminals.some((x) => x.id === id)) drop(id);
  void nextTick(sync);
}, { immediate: true });

watch(() => props.scheme, () => {
  for (const screen of screens.values()) screen.term.options.theme = terminalTheme(screen.host);
});

watch(size, (next) => {
  setTerminalSize(next);
  for (const screen of screens.values()) {
    screen.term.options.fontSize = next;
    fitScreen(screen);
  }
});

// The first tab's screen is built once the layer itself is in the document,
// which is also when its element has a box to fit to.
onMounted(() => void nextTick(sync));

onBeforeUnmount(() => {
  stopReading?.();
  for (const id of [...screens.keys()]) drop(id);
});

const clear = (id: string) => screens.get(id)?.term.clear();

const reconnect = (id: string) => {
  const next = { ...exits.value };
  delete next[id];
  exits.value = next;
  // The round is what the watcher reads; asking it to read again from here too
  // would open the stream twice for one press.
  rounds.value = { ...rounds.value, [id]: (rounds.value[id] ?? 0) + 1 };
};

const restart = async (id: string) => {
  failures.value = { ...failures.value, [id]: "" };
  try {
    await props.onRestart(id);
    reconnect(id);
  } catch (e) {
    failures.value = { ...failures.value, [id]: reason(e) };
  }
};

const copyAll = async (id: string) => {
  const buffer = screens.get(id)?.term.buffer.active;
  if (!buffer) return;
  const lines: string[] = [];
  for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
  try {
    await navigator.clipboard.writeText(lines.join("\n").replace(/\n+$/, ""));
  } catch (e) {
    props.onError(e);
  }
};

const changeSize = (change: SizeChange) => {
  const step = change === "larger" ? 1 : -1;
  size.value = change === "reset"
    ? TERMINAL_SIZES.standard
    : Math.min(TERMINAL_SIZES.max, Math.max(TERMINAL_SIZES.min, size.value + step));
};

const pointed = (id: string, e: MouseEvent) => {
  const screen = screens.get(id);
  if (!screen || !screen.term.hasSelection()) return;
  const text = screen.term.getSelection();
  if (!text.trim()) return;
  const at = screen.host.getBoundingClientRect();
  say(id, { x: e.clientX - at.left, y: e.clientY - at.top, text });
};

const copy = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    props.onError(e);
  }
};

const shellTag = (info: TerminalInfo) => props.shells.find((s) => s.path === info.shell)?.kind ?? info.name;
</script>

<template>
  <div
    v-for="info in terminals"
    :key="info.id"
    class="term"
    :hidden="shownId !== info.id"
    :data-scheme="scheme"
    :aria-label="t('terminal.name')"
  >
    <div v-if="note" class="term-note" role="note">
      <StudioIcon name="shield" />
      <p>{{ t("terminal.not_the_models") }}</p>
      <button type="button" data-action="terminal.note-dismiss" @click="seeNote(); note = false">{{ t("common.got_it") }}</button>
    </div>
    <TerminalHead
      :shell="shellTag(info)"
      :dir="info.dir"
      :shells="shells"
      :on-load-shells="onLoadShells"
      :on-shell="onShell"
      :on-clear="() => clear(info.id)"
      :on-reconnect="() => reconnect(info.id)"
      :on-copy-all="() => void copyAll(info.id)"
      :on-size="changeSize"
    />
    <div
      class="term-body"
      data-action-mouseup="terminal.select"
      data-action-mousedown="layer.dismiss"
      data-action-keydown="layer.dismiss"
      @mouseup="pointed(info.id, $event)"
      @mousedown="floatOf(info.id) && say(info.id, null)"
      @keydown="say(info.id, null)"
    >
      <div :ref="(el) => attach(info.id, el as Element | null)" class="term-screen" role="log" :aria-label="t('terminal.the_terminal')" />
      <div v-if="floatOf(info.id)" class="term-float" role="menu" :style="{ left: `${floatOf(info.id)!.x}px`, top: `${floatOf(info.id)!.y}px` }">
        <template v-if="floatOf(info.id)!.url">
          <button type="button" role="menuitem" data-action="terminal.open-link" @click="onOpenPage(floatOf(info.id)!.url!); say(info.id, null)">
            <StudioIcon name="globe" />{{ t("terminal.open_in_browser") }}
          </button>
          <button type="button" role="menuitem" data-action="terminal.copy-link" @click="void copy(floatOf(info.id)!.url!); say(info.id, null)">
            <StudioIcon name="copy" />{{ t("terminal.copy_link") }}
          </button>
        </template>
        <template v-else>
          <button v-if="onQuote" type="button" role="menuitem" data-action="terminal.send-to-chat"
            @click="onQuote(floatOf(info.id)!.text); screens.get(info.id)?.term.clearSelection(); say(info.id, null)">
            <StudioIcon name="quote" />{{ t("terminal.send_to_chat") }}
          </button>
          <button type="button" role="menuitem" data-action="terminal.copy-selection"
            @click="void copy(floatOf(info.id)!.text); screens.get(info.id)?.term.clearSelection(); say(info.id, null)">
            <StudioIcon name="copy" />{{ t("common.copy") }}
          </button>
        </template>
      </div>
    </div>
    <div v-if="failures[info.id]" class="workbench-error" role="alert">{{ failures[info.id] }}</div>
    <div v-if="ended(info) !== null" class="term-exit" role="status">
      <span>{{ t("terminal.exited", { code: ended(info) ?? 0 }) }}</span>
      <button type="button" data-primary="" data-action="terminal.restart" @click="void restart(info.id)">{{ t("terminal.restart") }}</button>
      <button type="button" data-action="terminal.close" @click="onClose(info.id)">{{ t("workbench.close_tab") }}</button>
    </div>
  </div>
</template>

<style scoped>
/* The screens share one column, and only the one in front takes it: a hidden
   tab is display:none, so a visible one has to fill the box the panel gave the
   layer rather than claim a height of its own. */
.term:not([hidden]) { flex: 1 1 auto; height: auto; min-height: 0; }
</style>
