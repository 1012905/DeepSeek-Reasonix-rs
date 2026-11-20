// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h } from "vue";
import { useSubmit } from "./usesubmit";
import { HttpError, type AgentPort } from "../port/port";
import { setRunningMessageIntent, type RunningMessageIntent } from "../state/prefs";

afterEach(() => { cleanup(); localStorage.clear(); vi.restoreAllMocks(); });

function mount(args: Parameters<typeof useSubmit>[0]) {
  let send!: ReturnType<typeof useSubmit>;
  render(defineComponent({
    setup() {
      send = useSubmit(args);
      return () => h("div");
    },
  }));
  return send;
}

const hook = (submit: () => Promise<unknown>) => {
  const dispatch = vi.fn();
  const fail = vi.fn();
  const port = { submit, queueFollowup: vi.fn(), steer: vi.fn() } as unknown as AgentPort;
  const send = mount({ port, running: false, dispatch, trajDispatch: vi.fn(), refreshStatus: vi.fn(), fail });
  return { send, dispatch, fail };
};

describe("useSubmit", () => {
  it("says a `!` command was refused for this connection and takes the row back", async () => {
    const { send, dispatch, fail } = hook(async () => { throw new HttpError(403, "no shell", { code: "shell.unavailable_over_http" }); });
    expect(await send("!ls")).toBe("shell_unavailable");
    expect(dispatch.mock.calls.some(([a]) => (a as { kind: string }).kind === "__unsent")).toBe(true);
    expect(fail).toHaveBeenCalledTimes(1);
  });

  it("keeps any other refusal a plain not-sent", async () => {
    const { send } = hook(async () => { throw new HttpError(409, "taken", { code: "provider.name_taken" }); });
    expect(await send("hello")).toBe(false);
  });

  it("answers true once the kernel took the line", async () => {
    const { send } = hook(async () => undefined);
    expect(await send("hello")).toBe(true);
  });

  it.each<RunningMessageIntent>(["steer", "followup"])("routes a running message through the selected canonical intent: %s", async (intent) => {
    setRunningMessageIntent(intent);
    const port = {
      steer: vi.fn(async () => ({ itemId: "steer", disposition: "steer_accepted" })),
      queueFollowup: vi.fn(async () => ({ itemId: "followup", disposition: "queued_followup" })),
      submit: vi.fn(),
    };
    const dispatch = vi.fn();
    const send = mount({ port: port as unknown as AgentPort, running: true, dispatch, trajDispatch: vi.fn(), refreshStatus: vi.fn(), fail: vi.fn() });
    expect(await send("later")).toBe(true);
    expect(port.steer).toHaveBeenCalledTimes(intent === "steer" ? 1 : 0);
    expect(port.queueFollowup).toHaveBeenCalledTimes(intent === "followup" ? 1 : 0);
    expect(port.submit).not.toHaveBeenCalled();
    expect(dispatch).toHaveBeenCalledWith(expect.objectContaining({ kind: "__queued", queued: intent }));
  });

  it("uses a changed preference on the next message without remounting", async () => {
    const port = { steer: vi.fn(), queueFollowup: vi.fn() };
    const send = mount({ port: port as unknown as AgentPort, running: true, dispatch: vi.fn(), trajDispatch: vi.fn(), refreshStatus: vi.fn(), fail: vi.fn() });
    await send("steer first");
    setRunningMessageIntent("followup");
    await send("follow up next");
    expect(port.steer).toHaveBeenCalledWith("steer first");
    expect(port.queueFollowup).toHaveBeenCalledWith("follow up next", undefined);
  });

  it.each<RunningMessageIntent>(["steer", "followup"])("leaves idle submission unchanged with %s selected", async (intent) => {
    setRunningMessageIntent(intent);
    const submit = vi.fn(async () => undefined);
    const { send } = hook(submit);
    expect(await send("idle")).toBe(true);
    expect(submit).toHaveBeenCalledWith("idle", undefined);
  });

  it("keeps skill invocations as follow-ups even with steering selected", async () => {
    const port = { steer: vi.fn(), queueFollowup: vi.fn(async () => ({ itemId: "skill" })) };
    const dispatch = vi.fn();
    const send = mount({ port: port as unknown as AgentPort, running: true, dispatch, trajDispatch: vi.fn(), refreshStatus: vi.fn(), fail: vi.fn() });
    const chips = { submit: "review", invocations: [] };
    await send("review", chips);
    expect(port.steer).not.toHaveBeenCalled();
    expect(port.queueFollowup).toHaveBeenCalledWith("review", chips);
    expect(dispatch).toHaveBeenCalledWith(expect.objectContaining({ kind: "__queued", queued: "followup" }));
  });

  it("uses the canonical receipt when an undeliverable steer becomes a follow-up", async () => {
    const port = { steer: vi.fn(async () => ({ itemId: "converted", disposition: "queued_followup" })) };
    const dispatch = vi.fn();
    const send = mount({ port: port as unknown as AgentPort, running: true, dispatch, trajDispatch: vi.fn(), refreshStatus: vi.fn(), fail: vi.fn() });
    await send("just missed the turn");
    expect(dispatch).toHaveBeenCalledWith(expect.objectContaining({ kind: "__queued", queued: "followup" }));
  });
});