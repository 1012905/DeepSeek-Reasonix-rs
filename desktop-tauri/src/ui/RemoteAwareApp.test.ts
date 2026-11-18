// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/vue";
import "./testkit";
import { boot, t } from "../i18n";
import type { HubPort } from "../port/hub";
import type { RemoteEnd } from "../port/cloud_link";

const state = vi.hoisted(() => ({
  stale: false,
  ended: null as RemoteEnd | null,
  signIn: vi.fn(),
  home: vi.fn(),
}));

vi.mock("./App.vue", async () => {
  const { defineComponent, h } = await import("vue");
  return { default: defineComponent({ name: "AppStub", props: { hub: { type: Object, required: true } }, setup: () => () => h("div", { class: "app-stub" }) }) };
});
vi.mock("./stale_page", async () => {
  const { ref } = await import("vue");
  return { useStalePage: () => ref(state.stale) };
});
vi.mock("../port/cloud_remote", () => ({
  remoteConnectionEnded: () => state.ended,
  onRemoteConnectionEnded: () => () => {},
  signInAgain: state.signIn,
  openRemoteHome: state.home,
}));

const RemoteAwareApp = (await import("./RemoteAwareApp.vue")).default;

beforeEach(() => { boot(); state.stale = false; state.ended = null; state.signIn.mockClear(); state.home.mockClear(); });
afterEach(() => cleanup());

const draw = (remote: string | null) => render(RemoteAwareApp, { props: { hub: {} as HubPort, remote } });

describe("the remote-aware app root", () => {
  it("draws the app with no notice when there is no remote device", () => {
    const { container } = draw(null);
    expect(container.querySelector(".app-stub")).not.toBeNull();
    expect(container.querySelector(".remote-closed")).toBeNull();
  });

  it("offers a refresh when a hosted page is an older build", () => {
    state.stale = true;
    const { container } = draw("dev-1");
    const notice = container.querySelector(".remote-closed.info")!;
    expect(notice).not.toBeNull();
    expect(notice.getAttribute("aria-label")).toBe(t("app.this_page_is_not_the"));
    expect(notice.querySelector("b")!.textContent).toBe(t("app.this_page_is_not_the"));
    expect(notice.querySelector("button")!.textContent).toBe(t("app.refresh_page"));
  });

  it("asks to sign in again when the relay ends the session for reauth", async () => {
    state.ended = { kind: "reauth", reason: "too old" };
    const { container } = draw("dev-1");
    const notice = container.querySelector(".remote-closed:not(.info)")!;
    expect(notice.getAttribute("aria-label")).toBe(t("app.sign_in_again"));
    expect(notice.querySelector("button")!.textContent).toBe(t("app.sign_in"));
    await fireEvent.click(notice.querySelector("button")!);
    expect(state.signIn).toHaveBeenCalledWith("dev-1");
  });

  it("says the kernel's own failure when the connection ends with a cause", () => {
    state.ended = { kind: "ended", reason: "offline", failure: { code: "offline" } };
    const { container } = draw("dev-1");
    const notice = container.querySelector(".remote-closed:not(.info)")!;
    expect(notice.getAttribute("aria-label")).toBe(t("remote.youre_offline"));
    expect(notice.querySelector("button")!.textContent).toBe(t("app.reconnect"));
  });

  it("lets the ended notice replace the stale one rather than stacking", () => {
    state.stale = true;
    state.ended = { kind: "ended", reason: "idle", failure: { code: "idle" } };
    const { container } = draw("dev-1");
    expect(container.querySelector(".remote-closed.info")).toBeNull();
    expect(container.querySelectorAll(".remote-closed").length).toBe(1);
  });
});