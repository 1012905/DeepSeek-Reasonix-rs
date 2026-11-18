import { describe, expect, it } from "vitest";
import { firstRunScreen, type FirstRunState } from "./firstRunScreen";

const base: FirstRunState = { settled: true, deferred: false, setupRequired: false, forceGuide: false, welcomed: true };
const screen = (over: Partial<FirstRunState>) => firstRunScreen({ ...base, ...over });

describe("the first-run surface decision", () => {
  it("holds the application until both machine reads have answered", () => {
    expect(screen({ settled: false, welcomed: false, setupRequired: true })).toBe("app");
  });

  it("asks for the key whenever one is owed, even on a machine that has met the app", () => {
    expect(screen({ setupRequired: true, welcomed: true })).toBe("guide");
    expect(screen({ setupRequired: true, welcomed: false })).toBe("guide");
  });

  it("shows the welcome scene to a configured machine that has never met the app", () => {
    expect(screen({ setupRequired: false, welcomed: false })).toBe("welcome");
  });

  it("opens the guide from the welcome scene when the person asks to set up", () => {
    expect(screen({ setupRequired: false, welcomed: false, forceGuide: true })).toBe("guide");
  });

  it("stands aside for this launch once the guide is put off", () => {
    expect(screen({ setupRequired: true, deferred: true })).toBe("app");
    expect(screen({ welcomed: false, deferred: true })).toBe("app");
  });

  it("is the application for a configured machine that has met the app", () => {
    expect(screen({})).toBe("app");
  });
});