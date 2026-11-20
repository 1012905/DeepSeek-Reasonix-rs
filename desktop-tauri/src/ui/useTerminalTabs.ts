import { computed, ref, shallowRef, toValue, watch, type MaybeRefOrGetter } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { host } from "../port/host";
import type { AgentPort } from "../port/port";
import type { ShellChoice, TerminalInfo } from "../port/terminal";
import { openPage } from "./browser_open";
import { lastShell, rememberShell } from "./terminal_prefs";
import type { Surface } from "./workbench_tabs";

export type TerminalTab = Extract<Surface, { kind: "terminal" }>;

const POLL_MS = 2000;

type Setter<T> = (next: T | ((prev: T) => T)) => void;

interface Props {
  port: MaybeRefOrGetter<AgentPort>;
  shown: MaybeRefOrGetter<boolean>;
  pick: (key: string) => void;
  setDismissed: Setter<Set<string>>;
  onExternal: (url: string) => void;
  onError: (message: string) => void;
  // Counts the presses that ask for a terminal: the one most recently used, or a first one.
  request: MaybeRefOrGetter<number>;
}

/** The person's terminals as workbench tabs. The kernel holds the list, so a
 *  terminal opened from another window, or before this one reloaded, is a tab
 *  here too; a tab is named for the command last submitted while it is busy. */
export function useTerminalTabs({ port, shown, pick, setDismissed, onExternal, onError, request }: Props) {
  const list = ref<TerminalInfo[]>([]);
  const shells = ref<ShellChoice[]>([]);
  const commands = ref<Record<string, string>>({});
  let changed = 0;

  watch([() => toValue(port), () => toValue(shown)], (_v, _was, onCleanup) => {
    if (!toValue(shown)) return;
    const agent = toValue(port);
    let live = true;
    const read = () => {
      const since = changed;
      agent.terminals().then((next) => {
        if (!live || since !== changed) return;
        // Not even an update that hands back the same value: it costs the
        // component another render, and the poll runs for as long as the panel.
        if (JSON.stringify(list.value) === JSON.stringify(next)) return;
        list.value = next;
      }, () => {});
    };
    read();
    const timer = window.setInterval(read, POLL_MS);
    onCleanup(() => { live = false; window.clearInterval(timer); });
  }, { immediate: true });

  const loadShells = () => { toValue(port).terminalShells().then((next) => { shells.value = next; }, () => {}); };
  watch(() => toValue(shown), (on) => { if (on) loadShells(); }, { immediate: true });

  const add = (info: TerminalInfo) => {
    changed += 1;
    list.value = [...list.value.filter((x) => x.id !== info.id), info];
    setDismissed((v) => { const n = new Set(v); n.delete(`terminal:${info.id}`); return n; });
    pick(`terminal:${info.id}`);
  };

  const open = async (kind = lastShell()) => {
    try {
      const info = await toValue(port).openTerminal(kind, 0, 0);
      add(info);
      const used = kind || shells.value.find((s) => s.path === info.shell)?.kind;
      if (used) rememberShell(used);
    } catch (e) {
      onError(t("terminal.could_not_open", { why: reason(e) }));
    }
  };

  let handled = toValue(request);
  watch([() => toValue(request), list], () => {
    const asked = toValue(request);
    if (asked === handled) return;
    handled = asked;
    const latest = list.value.at(-1);
    if (latest) pick(`terminal:${latest.id}`);
    else void open();
  }, { immediate: true });

  const restart = async (id: string) => {
    const info = await toValue(port).restartTerminal(id);
    changed += 1;
    list.value = list.value.map((x) => (x.id === id ? info : x));
  };

  const openUrl = async (url: string) => {
    if (!host().drawsBrowserViews()) return onExternal(url);
    try {
      pick(`browser:${(await openPage(toValue(port), url)).target}`);
    } catch (e) {
      onError(reason(e));
    }
  };

  const asking = shallowRef<{ anchor: HTMLElement; run: () => void } | null>(null);
  const cancelAsk = () => { asking.value = null; };
  const confirmAsk = () => { asking.value?.run(); asking.value = null; };
  // Closing a terminal with a command still running asks first; whichever way
  // the close was asked for, it goes through here.
  const guard = (targets: Surface[], run: (targets: Surface[]) => Promise<void>) => {
    const busy = targets.find((s) => s.kind === "terminal" && s.busy);
    if (!busy) return run(targets);
    const anchor = document.querySelector<HTMLElement>(`[data-key="terminal:${busy.kind === "terminal" ? busy.id : ""}"]`) ?? document.body;
    asking.value = { anchor, run: () => void run(targets) };
    return Promise.resolve();
  };

  const submitted = (id: string, line: string) => { commands.value = { ...commands.value, [id]: line }; };

  const tabs = computed<TerminalTab[]>(() => list.value.map((info, i) => ({
    kind: "terminal", id: info.id, busy: info.busy,
    name: info.busy && commands.value[info.id] ? commands.value[info.id] : list.value.length === 1 ? t("terminal.name") : t("terminal.n", { n: i + 1 }),
  })));

  return { terminals: list, tabs, shells, loadShells, open, restart, openUrl, submitted, asking, cancelAsk, confirmAsk, guard };
}