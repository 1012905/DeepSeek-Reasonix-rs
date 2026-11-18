// @vitest-environment jsdom
import { defineComponent, h, isRef, nextTick, ref, type Ref } from "vue";
import { cleanup, render, waitFor } from "@testing-library/vue";
import { afterEach, expect, it, vi } from "vitest";
import { MockPort } from "../../port/mock";
import { HttpError } from "../../port/port";
import { useDocument } from "./useDocument";

// @testing-library/vue has no renderHook; a harness component runs the
// composable in setup, with getters so a rerender carries new arguments.
function renderHook<T>(callback: (props: Ref<any>) => T, options?: { initialProps?: any }) {
  const props = ref(options?.initialProps ?? {});
  let out: T;
  const Harness = defineComponent({ setup() { out = callback(props); return () => h("div"); } });
  const utils = render(Harness);
  return {
    result: { get current(): any { return isRef(out) ? (out as any).value : out; } },
    rerender: async (p: any) => { props.value = p; await nextTick(); },
    unmount: () => utils.unmount(),
  };
}

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("discards a previous file's late response and aborts on close", async () => {
  const port = new MockPort();
  let done!: (value: { data: ArrayBuffer; version: string }) => void;
  let signal: AbortSignal | undefined;
  vi.spyOn(port, "workspacePreviewContent").mockImplementationOnce((_ref, s?: AbortSignal) => {
    signal = s;
    return new Promise((resolve) => { done = resolve; });
  });
  const { result, rerender, unmount } = renderHook((p) => useDocument(port, () => p.value.path, true, ""), { initialProps: { path: "one.csv" } });
  await waitFor(() => expect(done).toBeDefined());
  rerender({ path: "two.csv" });
  await waitFor(() => expect(result.current.value?.info.ref.path).toBe("two.csv"));
  expect(signal?.aborted).toBe(true);
  done({ data: new ArrayBuffer(0), version: "old" });
  await nextTick();
  expect(result.current.value?.info.ref.path).toBe("two.csv");
  unmount();
});

it("removes stale bytes when access is revoked and reload fails", async () => {
  const port = new MockPort();
  const { result } = renderHook(() => useDocument(port, "report.csv", true, ""));
  await waitFor(() => expect(result.current.value?.data).toBeTruthy());
  vi.spyOn(port, "workspacePreviewInfo").mockRejectedValue(new HttpError(403, "denied", { code: "workspace.preview_denied" }));
  result.current.reload();
  await waitFor(() => expect(result.current.value?.error).toBeTruthy());
  expect(result.current.value?.data).toBeUndefined();
});

it("checks metadata on invalidation and only forces unchanged bytes for manual refresh", async () => {
  const port = new MockPort();
  const content = vi.spyOn(port, "workspacePreviewContent");
  const info = vi.spyOn(port, "workspacePreviewInfo");
  const { result, rerender } = renderHook((p) => useDocument(port, "report.csv", true, () => p.value.refresh), { initialProps: { refresh: "first" } });
  await waitFor(() => expect(result.current.value?.data).toBeTruthy());
  rerender({ refresh: "tool finished" });
  await waitFor(() => expect(info).toHaveBeenCalledTimes(2));
  expect(content).toHaveBeenCalledTimes(1);
  result.current.reload();
  await waitFor(() => expect(content).toHaveBeenCalledTimes(2));
});