// @vitest-environment jsdom
import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "../../../i18n";
import type { HubPort } from "../../../port/hub";
import { MockHub } from "../../../port/mock_hub";
import type { AgentPort } from "../../../port/port";
import RemotePhone from "./data-remote-phone.vue";

function fakePort() {
  let status = { enabled: false, online: false, deviceId: "" };
  return {
    relayStatus: vi.fn(async () => ({ ...status })),
    relayEnable: vi.fn(async () => {
      status = { enabled: true, online: true, deviceId: "dev-9f2a1c4d8e6b" };
      return { deviceId: status.deviceId };
    }),
    relayDisable: vi.fn(async () => {
      status = { ...status, enabled: false, online: false };
    }),
    relayPair: vi.fn(async () => ({ code: "123456", expiresIn: 300 })),
  };
}

const opened: ReturnType<typeof mount>[] = [];
function card(port: ReturnType<typeof fakePort>) {
  const wrapper = mount(RemotePhone, {
    props: { hub: new MockHub() as unknown as HubPort, port: port as unknown as AgentPort, onError: () => {} },
  });
  opened.push(wrapper);
  return wrapper;
}

afterEach(() => { while (opened.length > 0) opened.pop()?.unmount(); });

describe("the web access block", () => {
  it("turns the relay on and then shows a pairing code", async () => {
    const port = fakePort();
    const wrapper = card(port);
    await flushPromises();
    expect(wrapper.find('[data-action="relay.enable"]').exists()).toBe(true);

    await wrapper.get('[data-action="relay.enable"]').trigger("click");
    await flushPromises();
    expect(port.relayEnable).toHaveBeenCalledOnce();
    expect(wrapper.get(".pstate").attributes("data-state")).toBe("use");
    expect(wrapper.text()).toContain(t("relay.online"));
    expect(wrapper.text()).toContain("dev-9f2a1c4d");

    await wrapper.get('[data-action="relay.pair"]').trigger("click");
    await flushPromises();
    expect(port.relayPair).toHaveBeenCalledOnce();
    const block = wrapper.get("[data-relay-code]");
    expect(block.text()).toContain("123456");
    expect(block.text()).toContain(t("relay.code_expires", { n: 5 }));
  });

  it("turns the relay off and withdraws the code", async () => {
    const port = fakePort();
    const wrapper = card(port);
    await flushPromises();
    await wrapper.get('[data-action="relay.enable"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-action="relay.pair"]').trigger("click");
    await flushPromises();
    expect(wrapper.find("[data-relay-code]").exists()).toBe(true);

    await wrapper.get('[data-action="relay.disable"]').trigger("click");
    await flushPromises();
    expect(port.relayDisable).toHaveBeenCalledOnce();
    expect(wrapper.find("[data-relay-code]").exists()).toBe(false);
    expect(wrapper.find('[data-action="relay.enable"]').exists()).toBe(true);
  });

  it("draws one phone block and one web-access block", async () => {
    const wrapper = card(fakePort());
    await flushPromises();
    expect(wrapper.findAll('[data-setting="phone"]').length).toBe(1);
    expect(wrapper.findAll('[data-setting="relay"]').length).toBe(1);
  });
});
