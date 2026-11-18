// @vitest-environment jsdom
//
// What this file proves: a view request belongs to one pane. Asking the active
// conversation for the workbench docks it alone, and re-opening an already-open
// conversation from the rail returns its pane to the reading view — the two
// behaviours the React window's per-pane viewRequests and useSidebarOpen carry
// (App.tsx:134,390) that a single shared counter collapses into every pane at once.
import { afterEach, describe, expect, it } from "vitest";
import { mount, type DOMWrapper, type VueWrapper } from "@vue/test-utils";
import { markRaw, nextTick } from "vue";
import "../testkit";
import App from "../App.vue";
import { MockHub } from "../../port/mock_hub";
import type { HubPort } from "../../port/hub";
import { boot, STORAGE } from "../../i18n";

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  sessionStorage.removeItem("rx-active-pane");
  boot();
});

const settle = async (times = 8) => { for (let i = 0; i < times; i++) await nextTick(); };

const app = (panes: number) => {
  const wrapper = mount(App, {
    props: { hub: markRaw(new MockHub(panes)) as unknown as HubPort },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return wrapper;
};

// The workbench column is drawn but hidden until the pane docks it, so its
// hidden attribute is the pane's own view state, read per pane rather than off
// the chrome, which only ever speaks for the active one.
const docked = (pane: Omit<DOMWrapper<Element>, "exists">) => pane.get('[data-pane="browser"]').attributes("hidden") === undefined;

describe("a pane's view request", () => {
  it("docks only the pane asked for, leaving the other conversation on its own view", async () => {
    boot();
    const wrapper = app(2);
    await settle();

    await wrapper.get('[data-action="pane.view"][data-value="browser"]').trigger("click");
    await settle();

    const panes = wrapper.findAll("section.pane");
    expect(panes.length).toBe(2);
    const active = panes.find((p) => p.attributes("data-active") === "")!;
    const other = panes.find((p) => p.attributes("data-active") !== "")!;
    expect(docked(active)).toBe(true);
    expect(docked(other)).toBe(false);
  });

  it("returns an already-open conversation's pane to the reading view when the rail re-opens it", async () => {
    boot();
    const wrapper = app(1);
    await settle();

    await wrapper.get('[data-action="pane.view"][data-value="browser"]').trigger("click");
    await settle();
    expect(docked(wrapper.get("section.pane"))).toBe(true);

    await wrapper.get('.sessrow[data-target="/sessions/mock.jsonl"]').trigger("click");
    await settle();
    expect(docked(wrapper.get("section.pane"))).toBe(false);
  });
});