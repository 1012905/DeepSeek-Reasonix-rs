import { describe, expect, it } from "vitest";
import { useSessionSwitch } from "./sessionswitch";

describe("which session is being opened", () => {
  it("keeps only the latest ticket able to land", () => {
    const sw = useSessionSwitch();
    const first = sw.begin("a");
    const second = sw.begin("b");
    expect(sw.current(first)).toBe(false);
    expect(sw.current(second)).toBe(true);
    expect(sw.opening("b")).toBe(true);
    expect(sw.opening("a")).toBe(false);
  });

  it("holds the pending mark until the pane lands, then clears on end", () => {
    const sw = useSessionSwitch();
    const ticket = sw.begin("a");
    expect(sw.pending).toBe(true);
    sw.landed(ticket);
    expect(sw.pending).toBe(false);
    sw.end(ticket);
    expect(sw.path).toBe("");
  });

  it("lets a superseded open end without clearing the newer one", () => {
    const sw = useSessionSwitch();
    const first = sw.begin("a");
    const second = sw.begin("b");
    sw.end(first);
    expect(sw.path).toBe("b");
    sw.end(second);
    expect(sw.path).toBe("");
  });

  it("cancels an in-flight open so it can no longer land", () => {
    const sw = useSessionSwitch();
    const ticket = sw.begin("a");
    sw.cancel();
    expect(sw.current(ticket)).toBe(false);
    expect(sw.path).toBe("");
    expect(sw.pending).toBe(false);
  });
});
