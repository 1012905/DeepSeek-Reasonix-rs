// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/vue";
import { defineComponent, h, nextTick } from "vue";
import Find from "./Find.vue";
import { t } from "../../i18n";
import { hitsIn, reading, useFindBar } from "./findBar";

afterEach(cleanup);

/** A transcript stand-in: rows the painter and the counter both walk. */
function transcript(rows: [string, string][]): HTMLElement {
  const host = document.createElement("div");
  host.className = "scroll";
  for (const [id, text] of rows) {
    const row = document.createElement("div");
    row.setAttribute("data-item", id);
    row.className = "enterbox";
    row.textContent = text;
    host.append(row);
  }
  document.body.append(host);
  return host;
}

describe("what a whole-word query means", () => {
  it("leaves the query alone until it is asked for", () => {
    expect(reading("cat", false)).toBe("cat");
    expect(reading("  cat  ", true)).toBe("(?<![0-9A-Za-z_])(?:cat)(?![0-9A-Za-z_])");
  });

  it("does not match a word inside a longer one", () => {
    const host = transcript([["a", "cat concat cats"]]);
    expect(hitsIn(host, "cat")).toHaveLength(3);
    expect(hitsIn(host, reading("cat", true), { regex: true })).toHaveLength(1);
  });

  it("reads punctuation literally rather than as a pattern", () => {
    const host = transcript([["a", "cost is $5 (roughly)"]]);
    expect(hitsIn(host, reading("$5 (roughly)", true), { regex: true })).toHaveLength(1);
  });
});

describe("counting a conversation", () => {
  it("counts each occurrence once, in reading order", () => {
    const host = transcript([["a", "one two one"], ["b", "one"]]);
    expect(hitsIn(host, "one").map((hit) => [hit.id, hit.nth])).toEqual([["a", 0], ["a", 1], ["b", 0]]);
  });

  it("keeps the case-sensitive reading apart from the folded one", () => {
    const host = transcript([["a", "Retry retry"]]);
    expect(hitsIn(host, "retry")).toHaveLength(2);
    expect(hitsIn(host, "retry", { caseSensitive: true })).toHaveLength(1);
  });

  it("finds nothing for an empty query", () => {
    expect(hitsIn(transcript([["a", "anything"]]), "  ")).toEqual([]);
  });
});

describe("the bar over the transcript", () => {
  it("shows a count once a query is typed, and steps through it", async () => {
    const host = transcript([["a", "one two one"], ["b", "one"]]);
    const { container, getByRole } = render(Find, { props: { open: true, host, onClose: () => {}, onError: () => {} } });
    const box = getByRole("searchbox") as HTMLInputElement;

    box.value = "one";
    box.dispatchEvent(new Event("input", { bubbles: true }));
    await waitFor(() => expect(container.querySelector(".tfind .n")?.textContent).toBe("1 / 3"));
    expect(container.querySelector(".tfind")?.hasAttribute("data-missed")).toBe(false);

    (container.querySelector('[data-value="next"]') as HTMLButtonElement).click();
    await waitFor(() => expect(container.querySelector(".tfind .n")?.textContent).toBe("2 / 3"));
  });

  it("says so when nothing matches, and names the reading that might have caused it", async () => {
    const host = transcript([["a", "one two"]]);
    const { container, getByRole } = render(Find, { props: { open: true, host, onClose: () => {}, onError: () => {} } });
    const box = getByRole("searchbox") as HTMLInputElement;

    (container.querySelector('[data-value="case"]') as HTMLButtonElement).click();
    box.value = "three";
    box.dispatchEvent(new Event("input", { bubbles: true }));
    await waitFor(() => expect(container.querySelector(".tfind")?.hasAttribute("data-missed")).toBe(true));
    expect(container.querySelector(".tfind .n")?.textContent).not.toBe("");
    // The advice names the switch that is on, which is the one worth turning off.
    expect(container.querySelector(".tfind-hint")?.textContent).toContain(t("chat.find_try_without_case"));
    expect(container.querySelector(".tfind-hint")?.textContent).toContain(t("chat.find_searched_everything"));
  });

  it("narrows the count when the query must match case, and restarts from the first hit", async () => {
    const host = transcript([["a", "Retry retry"]]);
    const { container, getByRole } = render(Find, { props: { open: true, host, onClose: () => {}, onError: () => {} } });
    const box = getByRole("searchbox") as HTMLInputElement;
    box.value = "retry";
    box.dispatchEvent(new Event("input", { bubbles: true }));
    await waitFor(() => expect(container.querySelector(".tfind .n")?.textContent).toBe("1 / 2"));

    (container.querySelector('[data-value="case"]') as HTMLButtonElement).click();
    await waitFor(() => expect(container.querySelector(".tfind .n")?.textContent).toBe("1 / 1"));
    expect(container.querySelector('[data-value="case"]')?.getAttribute("aria-pressed")).toBe("true");
  });

  it("closes on Escape and on its own button without telling the pane", async () => {
    const onClose = vi.fn();
    const host = transcript([["a", "one"]]);
    const { container, getByRole } = render(Find, { props: { open: true, host, onClose, onError: () => {} } });
    const box = getByRole("searchbox") as HTMLInputElement;

    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(onClose).toHaveBeenCalledOnce();

    (container.querySelector('[data-value="close"]') as HTMLButtonElement).click();
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("draws nothing while it is shut", () => {
    const { container } = render(Find, { props: { open: false, host: null, onClose: () => {}, onError: () => {} } });
    expect(container.querySelector(".tfind")).toBeNull();
  });

  it("survives a bar asked for before the pane has a scroll element", async () => {
    const { container, getByRole } = render(Find, { props: { open: true, host: null, onClose: () => {}, onError: () => {} } });
    const box = getByRole("searchbox") as HTMLInputElement;
    box.value = "one";
    box.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    expect(container.querySelector(".tfind .n")?.textContent).toBe("");
  });
});

describe("the bar's own state", () => {
  it("counts what the transcript drew, and steps through it in both directions", async () => {
    const host = transcript([["a", "one two one"]]);
    let bar: ReturnType<typeof useFindBar> | null = null;
    const Harness = defineComponent({
      setup() {
        bar = useFindBar(() => host, () => true);
        return () => h("div");
      },
    });
    render(Harness);
    await waitFor(() => expect(bar).not.toBeNull());

    bar!.ask("one");
    await waitFor(() => expect(bar!.total.value).toBe(2));
    expect(bar!.position.value).toBe(0);

    bar!.step(1);
    await waitFor(() => expect(bar!.position.value).toBe(1));
    // Wraps rather than stopping at the last hit.
    bar!.step(1);
    await waitFor(() => expect(bar!.position.value).toBe(0));
    bar!.step(-1);
    await waitFor(() => expect(bar!.position.value).toBe(1));
  });

  it("empties itself when the bar is shut", async () => {
    const host = transcript([["a", "one"]]);
    let bar: ReturnType<typeof useFindBar> | null = null;
    const Harness = defineComponent({
      setup() {
        bar = useFindBar(() => host, () => true);
        return () => h("div");
      },
    });
    render(Harness);
    bar!.ask("one");
    await waitFor(() => expect(bar!.total.value).toBe(1));
    bar!.reset();
    expect(bar!.query.value).toBe("");
    expect(bar!.total.value).toBe(0);
  });
});
