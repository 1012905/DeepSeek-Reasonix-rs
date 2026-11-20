// @vitest-environment jsdom
import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { t } from "../i18n";
import type { HubPort } from "../port/hub";
import { onRelayEnded, redeemAndConnect, RelayLinkError } from "../port/relay_cloud";
import RelayPair from "./RelayPair.vue";

vi.mock("./App.vue", async () => {
  const { h } = await import("vue");
  return {
    default: {
      name: "App",
      props: ["hub"],
      setup: () => () => h("div", { "data-stub-app": "" }),
    },
  };
});

vi.mock("../port/relay_cloud", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../port/relay_cloud")>();
  return {
    ...actual,
    relayBase: () => "https://relay.test",
    redeemAndConnect: vi.fn(),
    onRelayEnded: vi.fn(() => () => {}),
  };
});

const redeem = vi.mocked(redeemAndConnect);
const watchEnded = vi.mocked(onRelayEnded);
let ended: ((error: RelayLinkError) => void) | null = null;

beforeEach(() => {
  ended = null;
  redeem.mockReset();
  watchEnded.mockReset();
  watchEnded.mockImplementation((listener) => {
    ended = listener;
    return () => { ended = null; };
  });
});

const opened: ReturnType<typeof mount>[] = [];
const page = () => {
  const wrapper = mount(RelayPair);
  opened.push(wrapper);
  return wrapper;
};

afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

async function pair(wrapper: ReturnType<typeof mount>, code = "123456"): Promise<void> {
  await wrapper.get("input").setValue(code);
  await wrapper.get("form").trigger("submit");
  await flushPromises();
}

describe("the hosted pairing page", () => {
  it("asks for the code the desktop shows", () => {
    const wrapper = page();
    const input = wrapper.get("input");
    expect(wrapper.text()).toContain(t("relay.pair_title"));
    expect(wrapper.text()).toContain(t("relay.pair_hint"));
    expect(input.attributes("aria-label")).toBe(t("relay.code_label"));
    expect(input.attributes("inputmode")).toBe("numeric");
    expect(input.attributes("maxlength")).toBe("6");
    expect(input.attributes("autofocus")).toBeDefined();
    expect(wrapper.get("button[type=submit]").text()).toBe(t("relay.connect"));
  });

  it("keeps the form and names an invalid code", async () => {
    redeem.mockRejectedValueOnce(new RelayLinkError("invalid_code", "six digits"));
    const wrapper = page();
    await pair(wrapper, "000000");
    expect(wrapper.text()).toContain(t("relay.err_invalid_code"));
    expect(wrapper.find("input").exists()).toBe(true);
    expect(wrapper.find("[data-stub-app]").exists()).toBe(false);
  });

  it("mounts the app on a pairing and reports the connection", async () => {
    redeem.mockResolvedValueOnce({} as HubPort);
    const wrapper = page();
    await pair(wrapper);
    expect(redeem).toHaveBeenCalledWith("https://relay.test", "123456");
    expect(wrapper.find("[data-stub-app]").exists()).toBe(true);
    expect(wrapper.text()).toContain(t("relay.connected"));
    expect(watchEnded).toHaveBeenCalledOnce();
  });

  it("returns to the form when the desktop ends the session", async () => {
    redeem.mockResolvedValueOnce({} as HubPort);
    const wrapper = page();
    await pair(wrapper);
    expect(ended).not.toBeNull();
    ended?.(new RelayLinkError("ended", "closed"));
    await flushPromises();
    expect(wrapper.find("[data-stub-app]").exists()).toBe(false);
    expect(wrapper.find("input").exists()).toBe(true);
    expect(wrapper.text()).toContain(t("relay.err_ended"));
  });

  it("redeems over the network again after a link ended", async () => {
    redeem.mockResolvedValueOnce({} as HubPort);
    const wrapper = page();
    await pair(wrapper);
    ended?.(new RelayLinkError("ended", "closed"));
    await flushPromises();
    const native = globalThis.fetch;
    vi.stubGlobal("fetch", vi.fn());
    redeem.mockResolvedValueOnce({} as HubPort);
    await pair(wrapper, "654321");
    expect(redeem).toHaveBeenLastCalledWith("https://relay.test", "654321");
    expect(globalThis.fetch).toBe(native);
    vi.unstubAllGlobals();
  });
});
