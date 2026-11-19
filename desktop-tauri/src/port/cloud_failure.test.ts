// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { failureView, retryAfterSeconds, type RelayFailure } from "./cloud_failure";

describe("relay failure wording", () => {
  it("reads Retry-After only as a positive number of seconds", () => {
    expect(retryAfterSeconds("30")).toBe(30);
    expect(retryAfterSeconds("0")).toBeUndefined();
    expect(retryAfterSeconds("Wed, 21 Oct 2026 07:28:00 GMT")).toBeUndefined();
    expect(retryAfterSeconds(null)).toBeUndefined();
  });

  it("gives every code its own wording, with the wait only for rate limits", () => {
    const codes: RelayFailure[] = ["offline", "unreachable", "origin_rejected", "rate_limited", "refused", "idle", "unknown"];
    const titles = codes.map((code) => failureView({ code }).title);
    expect(new Set(titles).size).toBe(codes.length);
    expect(titles.every((title) => title.trim() !== "")).toBe(true);
    const bodies = codes.map((code) => failureView({ code }).body);
    expect(bodies.every((body) => body.trim() !== "")).toBe(true);
    expect(failureView({ code: "offline" }).retryWhenOnline).toBe(true);
    expect(failureView({ code: "refused" }).retryWhenOnline).toBe(false);
    expect(failureView({ code: "rate_limited", retryAfterS: 42 })).toMatchObject({ waitS: 42, action: "reconnect" });
    expect(failureView({ code: "rate_limited", retryAfterS: 9999 }).waitS).toBe(600);
    expect(failureView({ code: "refused", retryAfterS: 42 }).waitS).toBe(0);
  });

  it("does not offer a retry for the two failures a retry cannot fix", () => {
    expect(failureView({ code: "origin_rejected" }).action).toBe("ended");
    expect(failureView({ code: "refused" }).action).toBe("ended");
    for (const code of ["offline", "unreachable", "rate_limited", "idle", "unknown"] as const) {
      expect(failureView({ code }).action).toBe("reconnect");
    }
  });

  it("names the seconds it is waiting for, and asks for none when the limit named none", () => {
    expect(failureView({ code: "rate_limited", retryAfterS: 12 }).body).toContain("12");
    expect(failureView({ code: "rate_limited" }).body).not.toMatch(/\d+ s/);
  });
});
