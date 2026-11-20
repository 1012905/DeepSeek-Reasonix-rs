// @vitest-environment jsdom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import Group, { ApplyNote, ScopeTag } from "./Group.vue";

describe("settings block", () => {
  it("retains block identity, content and declared application timing", () => {
    const wrapper = mount(Group, {
      props: { id: "language", title: "Language", hint: "Choose language" },
      slots: { default: "control" },
    });
    expect(wrapper.attributes("id")).toBe("set-language");
    expect(wrapper.attributes("data-setting")).toBe("language");
    expect(wrapper.get(".grp-items").text()).toBe("control");
    expect(wrapper.get(".hint").attributes("title")).toBe("Choose language");
    expect(wrapper.get(".apply").attributes("data-apply")).toBe("restart");
    wrapper.unmount();
  });
  it("omits unknown metadata and usual scope labels", () => {
    const unknown = mount(ApplyNote, { props: { id: "not-declared" } });
    const usual = mount(ScopeTag, { props: { id: "esc-stop" } });
    expect(unknown.find(".apply").exists()).toBe(false);
    expect(usual.find(".scope-tag").exists()).toBe(false);
    unknown.unmount();
    usual.unmount();
  });
});
