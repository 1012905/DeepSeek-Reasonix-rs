// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/vue";
import "../testkit";
import * as hostModule from "../../port/host";
import Onboarding from "../Onboarding.vue";
import Welcome from "../Welcome.vue";
import OnboardingProject from "../OnboardingProject.vue";
import { MockPort } from "../../port/mock";
import { MockHub } from "../../port/mock_hub";
import type { AgentPort } from "../../port/port";
import type { HubPort, TreeWorkspace } from "../../port/hub";
import { boot, STORAGE } from "../../i18n";

afterEach(() => {
  cleanup();
  localStorage.setItem(STORAGE, "zh");
  boot();
  vi.restoreAllMocks();
});

// A machine that answers everything the guide asks and records what it was
// told. The reads stay real — the point is that the guide works against the
// kernel's own answers, not against a shape invented here.
function machine() {
  const port = new MockPort() as unknown as AgentPort;
  const hub = new MockHub() as unknown as HubPort;
  port.markWelcomed = vi.fn(async () => {});
  port.acknowledgeLaunchHealth = vi.fn(async () => {});
  // The probe decides what the next step can offer, so the answer is what the
  // guide is really being tested against.
  port.probeProvider = vi.fn(async () => ({
    kind: "openai", kinds: ["openai", "responses"], authHeader: true,
    models: ["deepseek-v4-pro", "deepseek-flash"], default: "deepseek-v4-pro",
    efforts: ["high"], effort: "high", vision: [], ambiguous: true, noProxy: false,
  }));
  hub.addWorkspace = vi.fn(async (path: string): Promise<TreeWorkspace> => ({ root: path, name: path.split("/").pop() ?? path, sessions: [] }));
  return { port, hub };
}

// The folder dialog belongs to the shell, and jsdom has none: this is the one
// stub in the file, and it answers only the question the guide asks of it.
function shellPicks(answer: () => Promise<string | null>) {
  vi.spyOn(hostModule.host(), "pickFolder").mockImplementation(answer);
}

const settle = (ms = 0) => new Promise((done) => setTimeout(done, ms));

describe("the first-run guide", () => {
  it("walks welcome → connect → model, asking for the key the machine has none of", async () => {
    const { port, hub } = machine();
    const done = vi.fn();
    render(Onboarding, { props: { hub, port, onDone: done, onError: () => {} } });

    await fireEvent.click(screen.getByRole("button", { name: "开始设置" }));
    await screen.findByText("连接一个模型服务");

    // A listed service has its address filled in; the key is the only thing to
    // type, and it is what this step exists to ask for.
    const key = document.getElementById("onb-key") as HTMLInputElement;
    expect(key).toBeTruthy();
    await fireEvent.update(key, "sk-test");
    await fireEvent.click(screen.getByRole("button", { name: "连接" }));

    await screen.findByText("选一个默认模型", undefined, { timeout: 5000 });
    expect(port.probeProvider).toHaveBeenCalled();
  });

  it("marks the welcome seen on every route out, and reports no project for one it never opened", async () => {
    const { port, hub } = machine();
    const done = vi.fn();
    render(Onboarding, { props: { hub, port, onDone: done, onError: () => {} } });

    await fireEvent.click(screen.getByRole("button", { name: "稍后再说" }));
    await waitFor(() => expect(done).toHaveBeenCalledWith({}), { timeout: 3000 });
    expect(port.markWelcomed).toHaveBeenCalledTimes(1);
  });

  it("leaves for the service settings from the step after the key, not from the one asking for it", async () => {
    const { port, hub } = machine();
    const openSettings = vi.fn();
    render(Onboarding, { props: { hub, port, onDone: () => {}, onError: () => {}, onOpenSettings: openSettings } });

    // Nothing wrong yet: the welcome explains itself and offers no way out.
    expect(screen.queryByRole("button", { name: "去连接" })).toBeNull();

    await fireEvent.click(screen.getByRole("button", { name: "开始设置" }));
    await screen.findByText("连接一个模型服务");
    expect(screen.queryByRole("button", { name: "去连接" })).toBeNull();

    // And back out of it: the sequence can be walked in both directions without
    // the machine being written to on the way.
    await fireEvent.click(screen.getByRole("button", { name: "返回" }));
    await screen.findByText("欢迎使用 Reasonix");
    expect(port.markWelcomed).not.toHaveBeenCalled();
  });
});

describe("the welcome", () => {
  it("offers both answers on the first frame rather than making the sequence a wait", async () => {
    const start = vi.fn();
    const skip = vi.fn();
    render(Welcome, { props: { onStart: start, onSkip: skip, seen: false } });

    await fireEvent.click(screen.getByRole("button", { name: "开始设置" }));
    expect(start).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole("button", { name: "稍后再说" }));
    expect(skip).toHaveBeenCalledTimes(1);
  });

  it("does not take a press on one of its own buttons as a dismissal", async () => {
    const start = vi.fn();
    const skip = vi.fn();
    render(Welcome, { props: { onStart: start, onSkip: skip, seen: true } });

    await fireEvent.pointerDown(screen.getByRole("button", { name: "开始设置" }));
    await settle(10);
    expect(skip).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
  });
});

describe("choosing a project", () => {
  it("takes the folder the picker answered with, and treats a dismissed dialog as an answer", async () => {
    const pick = vi.fn<() => Promise<string | null>>();
    pick.mockResolvedValueOnce("").mockResolvedValueOnce("/work/site");
    shellPicks(pick);

    const picked = vi.fn();
    render(OnboardingProject, { props: { onPick: picked, onSkip: () => {}, onError: () => {} } });
    const choose = screen.getByRole("button", { name: "选择文件夹…" });
    await fireEvent.click(choose);
    await settle();
    expect(picked).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();

    await fireEvent.click(choose);
    await waitFor(() => expect(screen.getByText("/work/site")).toBeTruthy());
    await fireEvent.click(screen.getByRole("button", { name: "开始" }));
    expect(picked).toHaveBeenCalledWith("/work/site");
  });

  it("shows a failure and hands it to the window rather than swallowing it", async () => {
    const failure = new Error("no such folder");
    shellPicks(async () => { throw failure; });
    const onError = vi.fn();
    render(OnboardingProject, { props: { onPick: () => {}, onSkip: () => {}, onError } });

    await fireEvent.click(screen.getByRole("button", { name: "选择文件夹…" }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(failure));
    expect(screen.getByRole("alert").textContent).toContain("no such folder");
  });
});
