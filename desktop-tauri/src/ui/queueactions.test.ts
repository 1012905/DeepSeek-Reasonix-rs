// @vitest-environment jsdom
import { effectScope, type EffectScope } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import "./testkit";
import { useQueueActions } from "./queueactions";
import { MockPort } from "../port/mock";
import type { AgentPort } from "../port/port";

const BODY = "把 gate_test.go 里那三个跳过的用例打开，再确认它们真的在自动检查里跑到了。";

const scopes: EffectScope[] = [];
afterEach(() => { for (const s of scopes.splice(0)) s.stop(); });

function setup(over: { readQueued?: (id: string) => Promise<string>; cancelQueued?: (id: string) => Promise<void> } = {}) {
  const port = new MockPort();
  port.readQueued = over.readQueued ?? (async () => BODY);
  port.cancelQueued = over.cancelQueued ?? vi.fn(async () => {});
  const dispatch = vi.fn();
  const fail = vi.fn();
  const scope = effectScope();
  scopes.push(scope);
  const actions = scope.run(() =>
    useQueueActions({ port: port as unknown as AgentPort, dispatch, fail, moved: 0 }),
  )!;
  return { port, dispatch, fail, actions };
}

describe("taking a queued line back", () => {
  it("hands the whole body back to the composer once the kernel has let go of it", async () => {
    const { actions, dispatch } = setup();
    await actions.onQueueCancel("i1");
    expect(dispatch).toHaveBeenCalledWith({ kind: "__unsent", id: "i1" });
    expect(actions.restored.value).toEqual({ n: 1, text: BODY });
  });

  it("keeps the entry when its body cannot be read, so nothing is destroyed", async () => {
    const cancelQueued = vi.fn(async () => {});
    const { actions, fail } = setup({
      readQueued: async () => {
        throw new Error("gone");
      },
      cancelQueued,
    });
    await actions.onQueueCancel("i1");
    expect(cancelQueued).not.toHaveBeenCalled();
    expect(actions.restored.value.n).toBe(0);
    expect(fail).toHaveBeenCalled();
  });

  it("does not restore a line the kernel refused to give up", async () => {
    const { actions, fail } = setup({
      cancelQueued: async () => {
        throw new Error("already delivered");
      },
    });
    await actions.onQueueCancel("i1");
    expect(actions.restored.value.n).toBe(0);
    expect(fail).toHaveBeenCalled();
  });
});