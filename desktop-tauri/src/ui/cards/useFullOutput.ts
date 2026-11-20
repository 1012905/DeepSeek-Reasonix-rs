import { computed, h, ref, type VNode, type VNodeChild } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import { count } from "../../i18n/format";
import type { ToolOutputPage } from "../../port/session";
import type { StateTool } from "../../state/outputcap";

export type ReadOutput = (callId: string, offset?: number, limit?: number) => Promise<ToolOutputPage>;

const PAGE = 64 * 1024;
// Four megabytes is more than a person reads in a card; past it the rest stays in the kernel.
const MAX_PAGES = 64;

type Fetch = { phase: "idle" } | { phase: "loading" } | { phase: "failed"; why: string } | { phase: "done"; text: string };

// A call whose middle the page let go of can bring it back whole: the pages are
// read from the kernel in turn and kept only by the card that asked.
export function useFullOutput(tool: StateTool, read?: ReadOutput): { text: string | undefined; note: VNode | null } {
  const got = ref<Fetch>({ phase: "idle" });
  const hidden = tool.hidden ?? 0;
  const load = async () => {
    if (!read || !tool.id || got.value.phase === "loading") return;
    got.value = { phase: "loading" };
    try {
      let text = "";
      let offset = 0;
      for (let i = 0; i < MAX_PAGES; i++) {
        const page = await read(tool.id, offset, PAGE);
        text += page.text;
        offset = page.offset + new TextEncoder().encode(page.text).length;
        if (!page.truncated) break;
      }
      got.value = { phase: "done", text };
    } catch (e) {
      got.value = { phase: "failed", why: reason(e) || t("chat.could_not_fetch_it_try") };
    }
  };
  const text = computed(() => (got.value.phase === "done" ? got.value.text : tool.output));
  const note = computed<VNode | null>(() => {
    const g = got.value;
    if (!(hidden > 0 && g.phase !== "done")) return null;
    const canLoad = !!read && !!tool.id;
    const children: VNodeChild[] = [t("chat.n_characters_from_the_middle", { n: count(hidden) })];
    if (canLoad && g.phase !== "loading") {
      children.push(h("button", { type: "button", class: "out-more", "data-action": "tool.show-full", onClick: () => void load() }, g.phase === "failed" ? t("common.retry") : t("chat.show_full_output")));
    }
    if (g.phase === "loading") children.push(h("span", { role: "status" }, ` ${t("chat.fetching")}`));
    if (g.phase === "failed") children.push(h("span", { role: "alert", class: "bad" }, ` ${g.why}`));
    return h("div", { class: "bound" }, children);
  });
  return {
    get text() { return text.value; },
    get note() { return note.value; },
  };
}