// @vitest-environment jsdom
import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "../../../i18n";
import type { HubPort } from "../../../port/hub";
import type { ShareStatus } from "../../../port/share";
import RemotePublic from "./data-remote-public.vue";

const base = (over: Partial<ShareStatus> = {}): ShareStatus => ({
  open: false,
  addresses: [],
  publicAddresses: [{ interface: "以太网", ip: "240e:338:448:1520::1", kind: "public" }],
  devices: [],
  port: 41234,
  ...over,
});

function fakeHub(initial: ShareStatus) {
  let status = initial;
  return {
    shareStatus: vi.fn(async () => ({ ...status, devices: [] })),
    openPublicShare: vi.fn(async (ip: string) => {
      status = base({ open: true, public: true, origin: `https://[${ip}]:41234`, certFingerprint: "ab12cd34ef56ab12cd34ef56ab12cd34" });
      return status;
    }),
    closeShare: vi.fn(async () => {
      status = base();
      return status;
    }),
  };
}

const opened: ReturnType<typeof mount>[] = [];
function card(hub: ReturnType<typeof fakeHub>) {
  const wrapper = mount(RemotePublic, { props: { hub: hub as unknown as HubPort, onError: () => {} } });
  opened.push(wrapper);
  return wrapper;
}

afterEach(() => { while (opened.length > 0) opened.pop()?.unmount(); });

describe("the direct door block", () => {
  it("opens the public listener and shows the origin, warning and fingerprint", async () => {
    const hub = fakeHub(base());
    const wrapper = card(hub);
    await flushPromises();

    await wrapper.get('[data-action="share.public-toggle"]').trigger("click");
    await flushPromises();
    expect(hub.openPublicShare).toHaveBeenCalledWith("240e:338:448:1520::1");
    expect(wrapper.text()).toContain("https://[240e:338:448:1520::1]:41234");
    expect(wrapper.text()).toContain(t("remote.browser_warns_about_certificate"));
    expect(wrapper.get('[data-action="share.public-fingerprint"]').text()).toContain("AB12CD34");
  });

  it("closes the door again", async () => {
    const hub = fakeHub(base());
    const wrapper = card(hub);
    await flushPromises();
    await wrapper.get('[data-action="share.public-toggle"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-action="share.public-toggle"]').trigger("click");
    await flushPromises();
    expect(hub.closeShare).toHaveBeenCalledOnce();
  });

  it("explains the missing public address instead of offering a switch", async () => {
    const hub = fakeHub(base({ publicAddresses: [] }));
    const wrapper = card(hub);
    await flushPromises();
    expect(wrapper.text()).toContain(t("remote.no_public_address"));
    expect(wrapper.find('[data-action="share.public-toggle"]').exists()).toBe(false);
  });
});
