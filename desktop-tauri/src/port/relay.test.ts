// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { SsePort } from "./sse";
import { MockPort } from "./mock";

afterEach(() => vi.unstubAllGlobals());

function stub(body: unknown, status = 200): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn(async () =>
    new Response(body === undefined ? null : JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function call(fetchMock: ReturnType<typeof vi.fn>, index = 0) {
  const [url, init] = fetchMock.mock.calls[index] as [string, RequestInit | undefined];
  return { url: String(url), method: init?.method ?? "GET", init: init ?? {} };
}

describe("the relay manager's routes", () => {
  it("reads the status with a GET and no body", async () => {
    const fetchMock = stub({ enabled: true, online: true, deviceId: "dev-1" });
    await expect(new SsePort().relayStatus()).resolves.toEqual({ enabled: true, online: true, deviceId: "dev-1" });
    const { url, method, init } = call(fetchMock);
    expect(url).toBe("/remote/relay");
    expect(method).toBe("GET");
    expect(init.credentials).toBe("same-origin");
  });

  it("turns the relay on, off and asks for a code with one POST each", async () => {
    const fetchMock = stub({ deviceId: "dev-1" });
    const port = new SsePort();

    await expect(port.relayEnable()).resolves.toEqual({ deviceId: "dev-1" });
    expect(call(fetchMock, 0)).toMatchObject({ url: "/remote/relay/enable", method: "POST" });
    expect(call(fetchMock, 0).init.body).toBeUndefined();

    await port.relayDisable();
    expect(call(fetchMock, 1)).toMatchObject({ url: "/remote/relay/disable", method: "POST" });

    fetchMock.mockResolvedValueOnce(new Response(JSON.stringify({ code: "482913", expiresIn: 300 }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }));
    await expect(port.relayPair()).resolves.toEqual({ code: "482913", expiresIn: 300 });
    expect(call(fetchMock, 2)).toMatchObject({ url: "/remote/relay/pair", method: "POST" });
  });

  it("reports a kernel without the route as off rather than failing the panel", async () => {
    stub({ error: { code: "not_found" } }, 404);
    await expect(new SsePort().relayStatus()).resolves.toEqual({ enabled: false, online: false, deviceId: "" });
  });

  it("keeps the kernel's refusal identity when turning it on fails", async () => {
    stub({ code: "relay.unavailable", error: "the relay is not configured" }, 503);
    const error = await new SsePort().relayEnable().then(() => null, (e: unknown) => e);
    expect(error).toMatchObject({ status: 503, reason: { code: "relay.unavailable" } });
  });
});

describe("the mock's relay", () => {
  it("starts off, enables a device, hands out a six-digit code, and turns off again", async () => {
    const port = new MockPort();

    await expect(port.relayStatus()).resolves.toEqual({ enabled: false, online: false, deviceId: "" });
    await expect(port.relayEnable()).resolves.toEqual({ deviceId: "mock-relay-device" });
    await expect(port.relayStatus()).resolves.toEqual({ enabled: true, online: true, deviceId: "mock-relay-device" });

    const code = await port.relayPair();
    expect(code.code).toMatch(/^\d{6}$/);
    expect(code.expiresIn).toBeGreaterThan(0);

    await port.relayDisable();
    await expect(port.relayStatus()).resolves.toEqual({ enabled: false, online: false, deviceId: "mock-relay-device" });
  });
});
