// @vitest-environment jsdom
import { effectScope, nextTick, ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { useDismiss, useEscape } from "./dismiss";

const escape = () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

describe("popover listener lifetime", () => {
  it("closes outside and stops listening when closed or disposed", async () => {
    const scope = effectScope();
    const open = ref(true);
    const box = ref(document.createElement("div"));
    const close = vi.fn();
    try {
      scope.run(() => useDismiss(open, box, close));
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      expect(close).toHaveBeenCalledTimes(1);
      open.value = false;
      await nextTick();
      escape();
      expect(close).toHaveBeenCalledTimes(1);
      open.value = true;
      await nextTick();
      escape();
      expect(close).toHaveBeenCalledTimes(2);
      scope.stop();
      escape();
      expect(close).toHaveBeenCalledTimes(2);
    } finally {
      scope.stop();
    }
  });

  it("restores the outer Escape layer after the inner scope ends", () => {
    const outer = effectScope();
    const inner = effectScope();
    const closeOuter = vi.fn();
    const closeInner = vi.fn();
    try {
      outer.run(() => useEscape(true, closeOuter));
      inner.run(() => useEscape(true, closeInner));
      escape();
      expect(closeInner).toHaveBeenCalledTimes(1);
      expect(closeOuter).not.toHaveBeenCalled();
      inner.stop();
      escape();
      expect(closeOuter).toHaveBeenCalledTimes(1);
      outer.stop();
      escape();
      expect(closeOuter).toHaveBeenCalledTimes(1);
    } finally {
      inner.stop();
      outer.stop();
    }
  });
});
