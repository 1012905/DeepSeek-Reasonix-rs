// @vitest-environment jsdom
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import "./testkit";
import { nextTick, type Component } from "vue";
import { t } from "../i18n";
import type { HubPort } from "../port/hub";
import type { Appearance as Look } from "../port/look";
import type { AgentPort } from "../port/port";
import PageIndex from "./PageIndex.vue";
import { FOLDS_INTO, NAV, SECTION_NAME, SETTINGS, pageHome, type Section } from "./prefsnav";
import type { SettingsProps } from "./SettingsProps";

// The sections are other agents' files and were still arriving when this was
// written. What this file is about is the shell around them — which page is
// shown, where `at` lands, what a search hit does — so the registry is replaced
// with one stub carrying the anchors these tests jump to. The stub keeps the
// part of the contract the shell depends on: it is a component taking a single
// `settingsProps` prop, and it can register a leave guard.
const state = vi.hoisted(() => ({ props: [] as unknown[], allowLeave: true }));

vi.mock("./settings/registry", async () => {
  const { defineComponent, h } = await import("vue");
  const { useSettingsLeaveGuard } = await import("./SettingsLeave");
  const Stub: Component = defineComponent({
    name: "StubSection",
    props: { settingsProps: { type: Object, required: true } },
    setup(props) {
      useSettingsLeaveGuard(() => (leave: () => void) => { if (state.allowLeave) leave(); });
      state.props.push(props.settingsProps);
      return () => h("div", { class: "stub-section", "data-section": "stub" }, [
        h("div", { id: "set-sandbox", "data-setting": "sandbox" }),
        h("div", { id: "set-shell", "data-setting": "shell" }),
        h("div", { id: "set-persona", "data-setting": "persona" }),
        h("div", { id: "set-language", "data-setting": "language" }),
      ]);
    },
  });
  return {
    SECTION_VIEW: {},
    // One section has no view on purpose: that is the placeholder path.
    sectionView: (section: Section) => (section === "display" ? undefined : Stub),
  };
});

const Settings = (await import("./Settings.vue")).default;

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  state.props.length = 0;
  state.allowLeave = true;
});

const props = (over: Partial<SettingsProps> = {}): SettingsProps => ({
  hub: {} as HubPort,
  onError: () => {},
  port: {} as AgentPort,
  status: null,
  theme: "auto",
  reloadThemes: () => {},
  onTheme: () => {},
  contrast: "",
  weight: "",
  onWeight: () => {},
  look: {} as Look,
  onLook: () => {},
  onContrast: () => {},
  onClose: () => {},
  onChanged: () => {},


  ...over,
});

const sheet = (over: Partial<SettingsProps> = {}) => {
  const wrapper = mount(Settings, { props: props(over), attachTo: document.body });
  opened.push(wrapper);
  return wrapper;
};

const query = (wrapper: VueWrapper) => wrapper.get<HTMLInputElement>(".prefs-find").element.value;

describe("the settings sheet", () => {
  it("is a titled dialog with a filter, a close and both panes", () => {
    const wrapper = sheet();
    const dialog = wrapper.get('[role="dialog"]');
    expect(dialog.attributes("aria-modal")).toBe("true");
    expect(dialog.attributes("aria-labelledby")).toBe("prefs-title");
    expect(wrapper.get("#prefs-title").text()).toBe(t("common.settings"));
    expect(wrapper.get(".prefs-find").attributes("placeholder")).toBe(t("settings.search_settings"));
    expect(wrapper.findAll('[data-action="settings.close"]').length).toBeGreaterThan(0);
    expect(wrapper.find(".prefs-nav").exists()).toBe(true);
    expect(wrapper.find(".prefs-main").exists()).toBe(true);
    expect(wrapper.find(".prefs-col .stub-section").exists()).toBe(true);
  });

  it("lists every nav entry with its icon and name, marking the page being shown", () => {
    const wrapper = sheet();
    for (const [group, rows] of NAV) {
      if (group) expect(wrapper.get(".prefs-nav").text()).toContain(t(group));
      for (const [id, name] of rows) {
        const row = wrapper.get(`#prefs-${id}`);
        expect(row.text(), id).toContain(t(name));
        expect(row.find("svg").exists(), id).toBe(true);
        expect(row.attributes("data-action")).toBe("settings.section");
      }
    }
    expect(wrapper.get("#prefs-session").attributes("aria-current")).toBe("page");
    expect(wrapper.get("#prefs-session").attributes("aria-selected")).toBe("true");
    expect(wrapper.get("#prefs-tools").attributes("aria-current")).toBeUndefined();
    expect(wrapper.findAll('[role="tab"]').length).toBe(NAV.flatMap(([, rows]) => rows).length);
  });

  it("switches page when a nav row is chosen", async () => {
    const wrapper = sheet();
    await wrapper.get("#prefs-tools").trigger("click");
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.get("#prefs-tools").attributes("aria-current")).toBe("page");
    expect(wrapper.get("#prefs-session").attributes("aria-current")).toBeUndefined();
  });

  it("hands each section the sheet's own props as one settingsProps object", () => {
    const wrapper = sheet({ theme: "dark" });
    expect(state.props.at(-1)).toBe(wrapper.props());
  });

  it("filters the catalogue as it is typed and jumps to the hit that was clicked", async () => {
    const wrapper = sheet();
    await wrapper.get(".prefs-find").setValue("sandbox");
    const hits = wrapper.findAll('.prefs-found [role="option"]');
    expect(hits.length).toBeGreaterThan(0);
    const hit = hits.find((row) => row.attributes("data-target") === "sandbox");
    expect(hit, "the sandbox row is among the hits").toBeTruthy();
    expect(hit!.text()).toContain(t("settings.sandbox"));
    expect(hit!.get(".where").text()).toContain(t(SECTION_NAME.tools ?? "tools"));

    await hit!.trigger("click");
    expect(wrapper.find(".prefs-found").exists()).toBe(false);
    expect(query(wrapper)).toBe("");
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.get("#prefs-tools").attributes("aria-current")).toBe("page");
    expect(wrapper.get("#set-sandbox").attributes("data-landed")).toBe("");
  });

  it("says so when a query finds nothing", async () => {
    const wrapper = sheet();
    await wrapper.get(".prefs-find").setValue("zzzz-nothing-is-called-this");
    expect(wrapper.get(".prefs-found .none").text()).toBe(t("settings.no_matching_setting"));
    expect(wrapper.findAll('.prefs-found [role="option"]').length).toBe(0);
  });

  it("opens at `section:anchor` with the block marked as landed", async () => {
    const wrapper = sheet({ at: "tools:sandbox" });
    // The landing is measured after the column has rendered.
    await nextTick();
    await nextTick();
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.get("#prefs-tools").attributes("aria-current")).toBe("page");
    expect(wrapper.get("#set-sandbox").attributes("data-landed")).toBe("");
    expect(wrapper.get("#set-shell").attributes("data-landed")).toBeUndefined();
  });

  it("reads an `at` that names a setting id the model used", async () => {
    const wrapper = sheet({ at: "sandbox.bash" });
    await nextTick();
    await nextTick();
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.get("#set-sandbox").attributes("data-landed")).toBe("");
  });

  it("keeps a section drawn inside another page on that page's row", async () => {
    const wrapper = sheet({ at: "providers:providers" });
    await nextTick();
    await nextTick();
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("providers");
    expect(wrapper.get("#prefs-model").attributes("aria-current")).toBe("page");
  });

  it("puts a page's own index under its title and lands on the entry chosen", async () => {
    const wrapper = sheet({ at: "tools:sandbox" });
    const index = wrapper.get(".page-index");
    expect(index.attributes("aria-label")).toBe(t("settings.on_this_page"));
    const entry = index.findAll("button").find((row) => row.attributes("data-target") === "shell");
    expect(entry, "the tools page lists its own blocks").toBeTruthy();
    await entry!.trigger("click");
    await nextTick();
    expect(wrapper.get("#set-shell").attributes("data-landed")).toBe("");
  });

  it("keeps a section with no view yet off the page and says so", () => {
    const wrapper = sheet({ at: "display" });
    const empty = wrapper.get("[data-section-empty]");
    expect(empty.attributes("data-section-empty")).toBe("display");
    expect(empty.text()).toContain(t("settings.not_in_this_version_yet"));
    expect(wrapper.find(".prefs-col .stub-section").exists()).toBe(false);
  });

  it("walks the results with the arrow keys and clears the query with Escape", async () => {
    const wrapper = sheet();
    const box = wrapper.get(".prefs-find");
    await box.setValue("sandbox");
    await box.trigger("keydown", { key: "ArrowDown" });
    const options = wrapper.findAll('.prefs-found [role="option"]');
    expect(document.activeElement).toBe(options[0].element);

    await options[0].trigger("keydown", { key: "Escape" });
    expect(query(wrapper)).toBe("");
    expect(wrapper.find(".prefs-found").exists()).toBe(false);
  });

  it("closes on Escape only while no section's guard is holding the sheet", async () => {
    const close = vi.fn();
    const wrapper = sheet({ onClose: close });
    await nextTick();

    state.allowLeave = false;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(close).not.toHaveBeenCalled();
    await wrapper.get('[data-action="settings.close"]').trigger("click");
    expect(close).not.toHaveBeenCalled();

    state.allowLeave = true;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("lets the search box keep its own Escape while it holds a query", async () => {
    const close = vi.fn();
    const wrapper = sheet({ onClose: close });
    const box = wrapper.get(".prefs-find");
    await box.setValue("sandbox");
    await box.trigger("keydown", { key: "Escape" });
    expect(close).not.toHaveBeenCalled();
    expect(query(wrapper)).toBe("");
    // With nothing left to clear, the same key reaches the sheet.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(close).toHaveBeenCalledTimes(1);
  });
});

describe("the page index", () => {
  const entry = (anchor: string) => ({ section: "tools" as Section, anchor, title: `settings.${anchor}`, scope: "machine" as const, apply: "immediate" as const });

  it("stays out of a page that holds fewer than three settings", () => {
    const two = mount(PageIndex, { props: { items: [entry("approval"), entry("rules")], onGo: () => {} } });
    expect(two.find(".page-index").exists()).toBe(false);
    const three = mount(PageIndex, { props: { items: [entry("approval"), entry("rules"), entry("sandbox")], onGo: () => {} } });
    expect(three.findAll(".page-index button").length).toBe(3);
    two.unmount();
    three.unmount();
  });
});

describe("what the sheet's rows and index are built from", () => {
  it("keeps every fold pointing at a page the nav can reach", () => {
    const rows = new Set<Section>(NAV.flatMap(([, items]) => items.map(([id]) => id)));
    for (const [from, into] of Object.entries(FOLDS_INTO)) {
      expect(pageHome(from as Section), from).toBe(into);
      expect(rows.has(into as Section) || rows.has(from as Section), `${from} folds into ${into}`).toBe(true);
    }
  });

  it("counts a folded page's settings under the row that draws them", () => {
    const drawn = (page: Section) => SETTINGS.filter((e) => !e.hidden && !e.within && pageHome(e.section) === page).length;
    expect(drawn("tools")).toBeGreaterThan(2);
    expect(drawn("model")).toBeGreaterThan(1);
    // Providers has a page of its own as well as folding, so it is counted once
    // on the row that shows it and never on a second row of its own.
    expect(NAV.flatMap(([, items]) => items.map(([id]) => id))).not.toContain("providers");
  });
});
