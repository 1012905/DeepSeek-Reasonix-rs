import { toValue, watch, type MaybeRefOrGetter } from "vue";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import { pageInFlight } from "./browser_open";

interface Opening {
  manual: MaybeRefOrGetter<boolean>;
  filesAsk: MaybeRefOrGetter<unknown>;
  port: MaybeRefOrGetter<AgentPort>;
  agentNow: MaybeRefOrGetter<string>;
  dropBlankStart: () => void;
  follow: (key: string) => void;
  current: () => () => boolean;
  setFailed: (message: string) => void;
}

/** What the workbench column does when it is opened. It follows the agent's page,
 *  and a page a followed link already has on its way; with neither it stays on
 *  its launcher until a browser is asked for from there. */
export function useColumnOpening({ manual, filesAsk, port, agentNow, dropBlankStart, follow, current, setFailed }: Opening) {
  let filesRequested = toValue(filesAsk);
  watch([() => toValue(manual), () => toValue(filesAsk), () => toValue(port)], (_v, _was, onCleanup) => {
    const ask = toValue(filesAsk);
    const files = filesRequested !== ask;
    filesRequested = ask;
    if (!toValue(manual) || files) return;
    const now = toValue(agentNow);
    if (now) {
      dropBlankStart();
      follow(`browser:${now}`);
      return;
    }
    const valid = current();
    let alive = true;
    pageInFlight(toValue(port))
      ?.then((tab) => { if (alive && valid()) follow(`browser:${tab.target}`); })
      .catch((e) => { if (alive && valid()) setFailed(reason(e)); });
    onCleanup(() => { alive = false; });
  }, { immediate: true });
}