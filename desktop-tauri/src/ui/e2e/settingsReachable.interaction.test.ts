// @vitest-environment jsdom
//
// What this file proves: a person can open the settings sheet, see every
// section it claims to have, change sections by pressing a row, find a setting
// by typing part of its title, and arrive at the block that setting is drawn
// in. The existing Settings.test.ts covers the shell against a stub registry;
// this one mounts the sheet with the registry that actually ships, so the pages
// behind the rows are the product's own and not a stand-in.
import { afterEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { markRaw, nextTick } from "vue";
import "../testkit";
import Settings from "../Settings.vue";
import { MockPort } from "../../port/mock";
import { MockHub } from "../../port/mock_hub";
import type { HubPort } from "../../port/hub";
import type { Appearance as Look } from "../../port/look";
import type { AgentPort } from "../../port/port";
import type { SettingsProps } from "../SettingsProps";
import { NAV, SECTION_NAME } from "../prefsnav";
import { t } from "../../i18n";

// The sheet takes a large set of ports. The fixture answers every one of them
// rather than a hand-written stub: a section that reads `port.schedules()` must
// get the fixture's board, not a cast of `{}` whose missing methods only show up
// as a page of "could not read this" fallbacks.
//
// markRaw because these objects are deep and reactive by accident otherwise;
// the sheet never mutates them.
const props = (over: Partial<SettingsProps> = {}): SettingsProps => ({
  hub: markRaw(new MockHub()) as unknown as HubPort,
  onError: () => {},
  port: markRaw(new MockPort()) as unknown as AgentPort,
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

const opened: VueWrapper[] = [];
afterEach(() => { while (opened.length > 0) opened.pop()?.unmount(); });

const sheet = (over: Partial<SettingsProps> = {}) => {
  const wrapper = mount(Settings, { props: props(over), attachTo: document.body });
  opened.push(wrapper);
  return wrapper;
};

/** Let a section's own reads settle and anything keyed on them re-render. */
const settle = async (times = 3) => { for (let i = 0; i < times; i++) await nextTick(); };

const navIds = NAV.flatMap(([, rows]) => rows.map(([id]) => id));

describe("the settings sheet a person actually opens", () => {
  it("lists every section the table of contents declares, by name, as a pressable row", () => {
    const wrapper = sheet();
    // The list is the product's own NAV, not a copy: a section added to the
    // table without a row on screen is a setting nobody can reach.
    expect(navIds.length).toBeGreaterThan(10);
    for (const id of navIds) {
      const row = wrapper.find(`#prefs-${id}`);
      expect(row.exists(), `#prefs-${id} is on the nav`).toBe(true);
      expect(row.attributes("role")).toBe("tab");
      expect(row.attributes("data-action")).toBe("settings.section");
      expect(row.text(), id).toContain(t(SECTION_NAME[id] ?? id));
    }
  });

  it("draws a real section view behind each row rather than the not-ported placeholder", async () => {
    const wrapper = sheet();
    for (const id of navIds) {
      await wrapper.get(`#prefs-${id}`).trigger("click");
      await settle();
      expect(wrapper.get(".prefs-main").attributes("data-sec"), id).toBe(id);
      expect(wrapper.get(`#prefs-${id}`).attributes("aria-current"), id).toBe("page");
      // Every section in the catalogue has a view, so none of them may fall
      // through to "not in this version yet".
      expect(wrapper.find(".prefs-col [data-section-empty]").exists(), `${id} has a view`).toBe(false);
      // And the view is not empty: the page behind the row draws at least one
      // setting block of its own.
      expect(wrapper.findAll(".prefs-col [data-setting]").length, `${id} draws settings`).toBeGreaterThan(0);
    }
  });

  it("switches from the user-behaviour page to the safety page when its row is pressed", async () => {
    const wrapper = sheet();
    // The session page is where the sheet opens; its own rows are about turns.
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("session");
    expect(wrapper.find("#set-plan-approval").exists()).toBe(true);

    await wrapper.get("#prefs-tools").trigger("click");
    await settle();

    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.find("#set-plan-approval").exists()).toBe(false);
    // The safety page's own blocks — not a placeholder, not the previous page.
    for (const anchor of ["approval", "sandbox", "shell", "rules"]) {
      expect(wrapper.find(`#set-${anchor}`).exists(), `#set-${anchor} is drawn`).toBe(true);
    }
  });

  it("finds a setting by a word in its title and lands the block it names", async () => {
    const wrapper = sheet();
    await wrapper.get(".prefs-find").setValue(t("common.language"));
    await settle();

    const hits = wrapper.findAll('.prefs-found [role="option"]');
    expect(hits.length).toBeGreaterThan(0);
    const hit = hits.find((row) => row.attributes("data-target") === "language");
    expect(hit, "the language setting is among the hits").toBeTruthy();
    expect(hit!.text()).toContain(t("common.language"));

    await hit!.trigger("click");
    await settle();

    expect(wrapper.find(".prefs-found").exists()).toBe(false);
    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("common");
    const landed = wrapper.get("#set-language");
    expect(landed.attributes("data-setting")).toBe("language");
  });

  it("finds a setting from another page and draws it on the page that owns it", async () => {
    const wrapper = sheet();
    // Typed from wherever the sheet happens to be open: the tools blocks are
    // not in the tree yet, so a hit that lands is a jump, not a stale match.
    expect(wrapper.find("#set-sandbox").exists()).toBe(false);
    await wrapper.get(".prefs-find").setValue(t("settings.sandbox"));
    await settle();

    const hit = wrapper.findAll('.prefs-found [role="option"]').find((row) => row.attributes("data-target") === "sandbox");
    expect(hit, "the sandbox setting is among the hits").toBeTruthy();
    expect(hit!.get(".where").text()).toContain(t(SECTION_NAME.tools ?? "tools"));

    await hit!.trigger("click");
    await settle();

    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.find("#set-sandbox").exists()).toBe(true);
    // The search jump really reached the sandbox control, not just its heading:
    // the two modes the control offers are in the DOM.
    expect(wrapper.findAll('[data-action="sandbox.mode"]').length).toBeGreaterThan(0);
  });

  it("opens straight at tools:sandbox when the window names that anchor", async () => {
    const wrapper = sheet({ at: "tools:sandbox" });
    await settle();

    expect(wrapper.get(".prefs-main").attributes("data-sec")).toBe("tools");
    expect(wrapper.get("#prefs-tools").attributes("aria-current")).toBe("page");
    expect(wrapper.find("#set-sandbox").exists()).toBe(true);
    // The landing is marked, so the block the window asked for is the one the
    // reader's eye is sent to.
    expect(wrapper.get("#set-sandbox").attributes("data-landed")).toBe("");
    expect(wrapper.find("#set-shell").attributes("data-landed")).toBeUndefined();
  });

  it("closes the sheet when Escape is pressed", async () => {
    const close = vi.fn();
    const wrapper = sheet({ onClose: close });
    await settle();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await settle();

    expect(close).toHaveBeenCalledTimes(1);
    // And the press is the sheet's own: nothing had to be clicked first.
    expect(wrapper.get(".prefs-find").attributes("aria-label")).toBe(t("settings.search_settings"));
  });
});
