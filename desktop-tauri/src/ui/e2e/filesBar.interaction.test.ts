// @vitest-environment jsdom
//
// What this file proves: the rail's files button answers to a pane. With no
// active session it tells the person to open one instead of doing nothing, and
// with a session open it closes a covering settings sheet before asking the
// pane for its files bar — the two side-effects the React window's openFiles
// carries (App.tsx:351) and the Vue port had dropped.
import { afterEach, describe, expect, it } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { markRaw, nextTick } from "vue";
import "../testkit";
import App from "../App.vue";
import { MockHub } from "../../port/mock_hub";
import type { HubPort } from "../../port/hub";
import { boot, STORAGE, t } from "../../i18n";

const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  sessionStorage.removeItem("rx-active-pane");
  boot();
});

const settle = async (times = 6) => { for (let i = 0; i < times; i++) await nextTick(); };

const app = (panes: number) => {
  const wrapper = mount(App, {
    props: { hub: markRaw(new MockHub(panes)) as unknown as HubPort },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return wrapper;
};

describe("the rail's files button", () => {
  it("asks the person to open a session first when no pane is active", async () => {
    boot();
    const wrapper = app(0);
    await settle();

    expect(wrapper.find(".errbar").exists()).toBe(false);
    await wrapper.get('[data-action="files.open"]').trigger("click");
    await settle();

    const bar = wrapper.get(".errbar");
    expect(bar.attributes("role")).toBe("alert");
    expect(bar.get("span").text()).toBe(t("app.open_a_session_first_to"));
  });

  it("closes a covering settings sheet before opening the files bar of the active pane", async () => {
    boot();
    const wrapper = app(1);
    await settle();

    await wrapper.get('[data-action="chrome.settings"]').trigger("click");
    await settle();
    expect(wrapper.get(".app").attributes("data-prefs")).toBe("");

    await wrapper.get('[data-action="files.open"]').trigger("click");
    await settle();

    expect(wrapper.get(".app").attributes("data-prefs")).toBeUndefined();
    expect(wrapper.find(".errbar").exists()).toBe(false);
  });
});