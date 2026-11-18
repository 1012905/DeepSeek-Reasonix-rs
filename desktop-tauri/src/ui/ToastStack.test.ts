// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/vue";
import { nextTick } from "vue";
import ToastStack from "./ToastStack.vue";
import { clearToasts, dismissToast, pushNote, pushOffer, pushToast, readToasts } from "../state/toasts";

afterEach(() => {
  clearToasts();
  cleanup();
  vi.useRealTimers();
});

const rows = (el: Element) => [...el.querySelectorAll<HTMLElement>(".toast")];

describe("the corner notices", () => {
  it("draws what state/toasts was told, and nothing before that", async () => {
    const { container } = render(ToastStack);
    expect(rows(container).length).toBe(0);

    pushToast("model switch failed");
    await nextTick();

    const shown = rows(container);
    expect(shown).toHaveLength(1);
    expect(shown[0].textContent).toContain("model switch failed");
    // A failure interrupts; a note waits. The distinction is carried here, not
    // by the caller, because the tone is what decides it.
    expect(shown[0].getAttribute("role")).toBe("alert");
    expect(shown[0].dataset.kind).toBe("error");
  });

  it("draws a note as a status with no failure colour", async () => {
    const { container } = render(ToastStack);
    pushNote("copied");
    await nextTick();

    const [note] = rows(container);
    expect(note.getAttribute("role")).toBe("status");
    expect(note.dataset.kind).toBe("note");
    expect(note.querySelector<HTMLElement>(".toast-icon")?.dataset.tone).toBe("ok");
  });

  it("never draws past the state's own ceiling", async () => {
    const { container } = render(ToastStack);
    for (const text of ["one failed", "two failed", "three failed", "four failed"]) pushToast(text);
    await nextTick();
    expect(rows(container)).toHaveLength(3);
    expect(container.textContent).toContain("four failed");
    expect(container.textContent).not.toContain("one failed");
  });

  it("takes a notice's button, and hangs up the notice with it", async () => {
    const run = vi.fn();
    const { container, getByText } = render(ToastStack);
    pushOffer("a newer build is ready", { label: "Restart now", run });
    await nextTick();

    expect(readToasts()).toHaveLength(1);
    getByText("Restart now").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();

    expect(run).toHaveBeenCalledOnce();
    expect(readToasts()).toHaveLength(0);
    expect(rows(container)).toHaveLength(0);
  });

  it("reports a button that throws instead of leaving the notice standing", async () => {
    const onError = vi.fn();
    const boom = new Error("restart refused");
    const { getByText } = render(ToastStack, { props: { onError } });
    pushOffer("a newer build is ready", { label: "Restart now", run: () => { throw boom; } });
    await nextTick();

    getByText("Restart now").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();

    expect(onError).toHaveBeenCalledWith(boom);
    expect(readToasts()).toHaveLength(0);
  });

  it("closes on its own after being read, and holds still while it is", async () => {
    vi.useFakeTimers();
    const { container } = render(ToastStack);
    pushToast("branch switch failed");
    await nextTick();
    // pointerenter does not bubble: it fires on the corner itself when the
    // pointer arrives over any part of it.
    const corner = container.querySelector<HTMLElement>(".toasts")!;

    // The pointer is on it: a notice that vanishes under the cursor takes the
    // button away from whoever was reaching for it.
    corner.dispatchEvent(new Event("pointerenter"));
    vi.advanceTimersByTime(60_000);
    await nextTick();
    expect(rows(container)).toHaveLength(1);

    corner.dispatchEvent(new Event("pointerleave"));
    vi.advanceTimersByTime(10_000);
    await nextTick();
    expect(rows(container)).toHaveLength(0);
  });

  it("gives a note a shorter stay than a failure", async () => {
    vi.useFakeTimers();
    const { container } = render(ToastStack);
    pushNote("copied");
    await nextTick();
    vi.advanceTimersByTime(5_000);
    await nextTick();
    expect(rows(container)).toHaveLength(0);
  });

  it("keeps the timer honest when a notice is dismissed by hand", async () => {
    vi.useFakeTimers();
    const { container } = render(ToastStack);
    pushToast("first failed");
    pushToast("second failed");
    await nextTick();
    dismissToast(readToasts()[0].id);
    await nextTick();
    expect(rows(container)).toHaveLength(1);
    // The one left still leaves on its own schedule.
    vi.advanceTimersByTime(10_000);
    await nextTick();
    expect(rows(container)).toHaveLength(0);
  });
});
