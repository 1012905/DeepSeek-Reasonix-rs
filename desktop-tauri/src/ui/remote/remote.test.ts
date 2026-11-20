// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/vue";
import { flushPromises } from "@vue/test-utils";
import "./../../ui/testkit";
import RemoteAsk from "./RemoteAsk.vue";
import RemoteDirs from "./RemoteDirs.vue";
import RemoteHosts from "./RemoteHosts.vue";
import RemoteProjectMenu from "./RemoteProjectMenu.vue";
import Remotes from "./Remotes.vue";
import type { RemoteAsk as Question, RemoteListing } from "../../port/remote";
import type { HubPort } from "../../port/hub";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const question = (over: Partial<Question> = {}): Question => ({
  epoch: "epoch-1",
  operationId: "op-1",
  askId: "ask-1",
  kind: "hostkey",
  host: "gpu-box",
  address: "10.0.0.4:22",
  keyType: "ssh-ed25519",
  fingerprint: "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  ...over,
});

describe("the first-connect fingerprint dialog", () => {
  it("shows what was presented and answers nothing until it is answered", () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question(), onDecide } });
    // The fingerprint is on screen to be compared, never pre-accepted.
    expect(screen.getByText(question().fingerprint!)).toBeTruthy();
    expect(screen.getByText("ssh-ed25519")).toBeTruthy();
    expect(onDecide).not.toHaveBeenCalled();
    expect(onDecide).not.toHaveBeenCalledWith(true, expect.anything(), expect.anything());
  });

  it("accepts only on an explicit press, and remembers only when asked to", async () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question(), onDecide } });
    await fireEvent.click(screen.getByRole("button", { name: "确认" }));
    expect(onDecide).toHaveBeenCalledWith(true, false, "");
  });

  it("carries the remember choice as its own answer", async () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question(), onDecide } });
    await fireEvent.click(screen.getByRole("checkbox"));
    await fireEvent.click(screen.getByRole("button", { name: "指纹一致，记住该机器" }));
    expect(onDecide).toHaveBeenCalledWith(true, true, "");
  });

  it("declines on Escape rather than leaving the dial waiting", async () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question(), onDecide } });
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onDecide).toHaveBeenCalledWith(false, false, "");
  });

  it("refuses to accept a key nobody could compare", () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question({ fingerprint: undefined, keyType: undefined, address: undefined }), onDecide } });
    expect(screen.getByRole("button", { name: "确认" })).toHaveProperty("disabled", true);
    expect(onDecide).not.toHaveBeenCalled();
  });

  it("takes a secret as the answer it is, and never as something to remember", async () => {
    const onDecide = vi.fn();
    render(RemoteAsk, { props: { ask: question({ kind: "passphrase", fingerprint: undefined, identityFile: "~/.ssh/id_ed25519" }), onDecide } });
    // A password field is not a textbox to the accessibility tree, which is
    // the point: a secret should not read as an ordinary field.
    const field = document.querySelector<HTMLInputElement>('.askcard input[type="password"]');
    expect(field).toBeTruthy();
    await fireEvent.update(field!, "hunter2");
    await fireEvent.click(screen.getByRole("button", { name: "继续" }));
    expect(onDecide).toHaveBeenCalledWith(true, false, "hunter2");
  });

  it("answers a fresh question without the previous answer's consent", async () => {
    const onDecide = vi.fn();
    const first = question();
    const { rerender } = render(RemoteAsk, { props: { ask: first, onDecide } });
    await fireEvent.click(screen.getByRole("checkbox"));
    await rerender({ ask: { ...question(), askId: "ask-2" }, onDecide });
    await fireEvent.click(screen.getByRole("button", { name: "确认" }));
    expect(onDecide).toHaveBeenCalledWith(true, false, "");
  });
});

const listing = (over: Partial<RemoteListing> = {}): RemoteListing => ({
  path: "/srv",
  parent: "/",
  folders: [{ name: "training", path: "/srv/training" }],
  ...over,
});

describe("browsing a folder on another machine", () => {
  // remoteDirs names the machine as well as the path: the folder is that
  // machine's, so a listing is never answered from a path alone.
  const portWith = (answer: (host: string, path?: string) => Promise<RemoteListing>) => ({ remoteDirs: vi.fn(answer) }) as unknown as HubPort;

  it("opens on the folder it was given and lists what the far machine answered", async () => {
    const hub = portWith(() => Promise.resolve(listing()));
    render(RemoteDirs, { props: { hub, host: "gpu-box", start: "/srv", onPick: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    expect(hub.remoteDirs).toHaveBeenCalledWith("gpu-box", "/srv");
    expect(screen.getByRole("button", { name: /training/ })).toBeTruthy();
  });

  it("picks the folder on screen rather than one typed and unconfirmed", async () => {
    const onPick = vi.fn();
    const hub = portWith(() => Promise.resolve(listing()));
    render(RemoteDirs, { props: { hub, host: "gpu-box", start: "/srv", onPick, onClose: vi.fn() } });
    await flushPromises();
    await fireEvent.click(screen.getByRole("button", { name: "使用此目录" }));
    expect(onPick).toHaveBeenCalledWith("/srv");
  });

  it("dials again for a subfolder instead of cutting the path here", async () => {
    const hub = portWith((_host, path) => Promise.resolve(path === "/srv/training" ? listing({ path: "/srv/training", parent: "/srv", folders: [] }) : listing()));
    render(RemoteDirs, { props: { hub, host: "gpu-box", start: "/srv", onPick: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    await fireEvent.click(screen.getByRole("button", { name: /training/ }));
    await flushPromises();
    expect(hub.remoteDirs).toHaveBeenLastCalledWith("gpu-box", "/srv/training");
    expect(screen.getByText("该目录下没有子目录 —— 它本身即可作为工作区")).toBeTruthy();
  });
});

const host = (over: Partial<import("../../port/remote").RemoteHost> = {}): import("../../port/remote").RemoteHost => ({
  name: "gpu-box",
  target: "ada@10.0.0.4",
  status: "idle",
  workspace: "/srv/training",
  workspaces: ["/srv/training"],
  ...over,
});

const row = (over: Partial<import("./RemoteHosts.vue").RemoteHostsProps> = {}) => ({
  host: host(),
  projects: ["/srv/training"],
  position: 0,
  total: 1,
  onProbe: vi.fn(),
  onOpen: vi.fn(),
  onEdit: vi.fn(),
  onRemove: vi.fn(),
  onToggleOff: vi.fn(),
  onHide: vi.fn(),
  ...over,
});

describe("one machine's row", () => {
  it("names the machine, its address and what its link is doing", () => {
    render(RemoteHosts, { props: row() });
    expect(screen.getByText("gpu-box")).toBeTruthy();
    expect(screen.getByText("ada@10.0.0.4")).toBeTruthy();
    expect(screen.getByText("未连接")).toBeTruthy();
  });

  it("says suspended rather than a link state when the machine is turned off", () => {
    render(RemoteHosts, { props: row({ host: host({ disabled: true, status: "connected" }) }) });
    expect(screen.getByText("已停用")).toBeTruthy();
  });

  it("asks and shows the answer, closed routes worded by their own codes", async () => {
    const probe = vi.fn();
    const { rerender } = render(RemoteHosts, { props: row({ onProbe: probe }) });
    await fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
    expect(probe).toHaveBeenCalledTimes(1);
    await rerender({
      ...row({ onProbe: probe }),
      probe: {
        os: "linux", arch: "arm64", home: "/home/ada", kernel: "reasonix", version: "1.2.3", ready: true,
        routes: [{ name: "npm", ok: true }],
      },
    });
    expect(screen.getByText(/linux\/arm64/)).toBeTruthy();
    expect(screen.getByText("reasonix 1.2.3")).toBeTruthy();
    expect(screen.getByText("可以连接 —— 远端已有内核，或可安装一个")).toBeTruthy();
  });

  it("asks before taking a machine out of the book", async () => {
    const onRemove = vi.fn();
    render(RemoteHosts, { props: row({ onRemove }) });
    await fireEvent.click(screen.getByRole("button", { name: /更多：/ }));
    await fireEvent.click(screen.getByRole("menuitem", { name: "移除" }));
    // The question comes first: removing a machine is not undone by pressing the
    // row it was on again.
    expect(onRemove).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onRemove).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: /更多：/ }));
    await fireEvent.click(screen.getByRole("menuitem", { name: "移除" }));
    await fireEvent.click(screen.getByRole("button", { name: "移除" }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});

describe("a remote project's menu", () => {
  it("offers the entries it has callbacks for and nothing it cannot do", async () => {
    const onRemove = vi.fn();
    render(RemoteProjectMenu, { props: { name: "training", root: "/srv/training", host: "gpu-box", defaultRoot: "/srv/training", busy: false, onRemove } });
    await fireEvent.click(screen.getByRole("button", { name: /项目操作/ }));
    // The default folder is already the default: promoting it again would be a
    // write that changes nothing.
    expect(screen.queryByRole("menuitem", { name: /设为默认/ })).toBeNull();
    await fireEvent.click(screen.getByRole("menuitem", { name: /从列表移除/ }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it("promotes another folder without a second field to disagree with", async () => {
    const onMakeDefault = vi.fn();
    render(RemoteProjectMenu, { props: { name: "api", root: "/srv/api", host: "gpu-box", defaultRoot: "/srv/training", busy: false, onMakeDefault } });
    await fireEvent.click(screen.getByRole("button", { name: /项目操作/ }));
    await fireEvent.click(screen.getByRole("menuitem", { name: /设为默认/ }));
    expect(onMakeDefault).toHaveBeenCalledTimes(1);
  });
});

describe("the machines panel", () => {
  const hubWith = (over: Partial<HubPort> = {}) => ({
    remoteHosts: vi.fn(async () => [host()]),
    remoteCandidates: vi.fn(async () => [] as string[]),
    wslDistros: vi.fn(async () => ({ available: false, distros: [] })),
    saveRemoteHost: vi.fn(async () => {}),
    removeRemoteHost: vi.fn(async () => {}),
    probeRemote: vi.fn(async () => ({ os: "linux", arch: "arm64", home: "/home/ada", ready: true, routes: [] })),
    placeRemoteHost: vi.fn(async () => {}),
    ...over,
  }) as unknown as HubPort;

  it("lists the book and offers a way to add to it", async () => {
    const hub = hubWith();
    render(Remotes, { props: { hub, onOpenRemote: vi.fn(), onError: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    expect(screen.getByText("gpu-box")).toBeTruthy();
    expect(screen.getByRole("button", { name: /添加机器/ })).toBeTruthy();
  });

  it("saves a new machine with the address it was given", async () => {
    const hub = hubWith();
    render(Remotes, { props: { hub, onOpenRemote: vi.fn(), onError: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    await fireEvent.click(screen.getByRole("button", { name: /添加机器/ }));
    await fireEvent.update(screen.getByPlaceholderText("gpu-box"), "new-box");
    await fireEvent.update(screen.getByPlaceholderText("10.0.0.4"), "10.0.0.9");
    // Two buttons say "添加" in this form: one lists a folder, the other saves
    // the machine. The action is what tells them apart.
    await fireEvent.click(document.querySelector('[data-action="remotes.add"]')!);
    await flushPromises();
    expect(hub.saveRemoteHost).toHaveBeenCalledWith(expect.objectContaining({ name: "new-box", host: "10.0.0.9" }));
  });

  it("imports an ssh alias with the name alone, and nothing else blanked", async () => {
    const hub = hubWith({ remoteCandidates: vi.fn(async () => ["lab"]) });
    render(Remotes, { props: { hub, onOpenRemote: vi.fn(), onError: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    await fireEvent.click(screen.getByRole("button", { name: /lab/ }));
    await flushPromises();
    expect(hub.saveRemoteHost).toHaveBeenCalledWith(expect.objectContaining({ name: "lab", useSSHConfig: true }));
  });

  it("opens a pane on a machine through the caller, never by itself", async () => {
    const onOpenRemote = vi.fn();
    const hub = hubWith();
    render(Remotes, { props: { hub, onOpenRemote, onError: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    await fireEvent.click(screen.getByRole("button", { name: /更多：/ }));
    await fireEvent.click(screen.getByRole("menuitem", { name: "打开" }));
    expect(onOpenRemote).toHaveBeenCalledWith({ host: "gpu-box", workspace: "/srv/training" });
  });

  it("says so once when the kernel does not do remote panes at all", async () => {
    const hub = hubWith({ remoteHosts: vi.fn(async () => null) });
    render(Remotes, { props: { hub, onOpenRemote: vi.fn(), onError: vi.fn(), onClose: vi.fn() } });
    await flushPromises();
    expect(screen.getByText("远程连接失败")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /添加机器/ })).toBeNull();
  });
});
