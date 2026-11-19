// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { remoteCodec } from "./cloud_link";
import {
  onRelayEnded,
  redeemAndConnect,
  relayBase,
  relayEnded,
  RelayLinkError,
  RemoteEventSource,
} from "./relay_cloud";
import { openLiveStream, STREAM_SILENCE_MS } from "./livestream";

interface Device {
  id: string;
  publicKey: string;
  privateKey: CryptoKey;
}

async function device(id: string): Promise<Device> {
  const pair = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]) as CryptoKeyPair;
  const publicKey = remoteCodec.bytesToBase64(new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey)));
  return { id, publicKey, privateKey: pair.privateKey };
}

// The desktop half of the encrypted session: the same X25519 -> HKDF ->
// AES-GCM the Go host runs, so what the page sends is really decrypted here.
class FakeDesktop {
  private key: CryptoKey | null = null;
  private info = new TextEncoder().encode("");
  readonly opened: Array<Record<string, unknown>> = [];

  constructor(
    readonly deviceId: string,
    private readonly privateKey: CryptoKey,
    private readonly socket: FakeSocket,
    // What the ready frame claims to be; a desktop that is not the paired one
    // would answer with another id here.
    private readonly claim: string = deviceId,
  ) {}

  async receive(wire: string) {
    if (this.key === null) {
      await this.greet(wire);
      return;
    }
    const frame = await this.open(wire);
    this.opened.push(frame);
    await this.seal({
      type: "desktop.response", id: frame.id, index: 0, done: true, status: 200,
      body: remoteCodec.bytesToBase64(new TextEncoder().encode(JSON.stringify({ ok: true }))),
    });
  }

  private async greet(wire: string) {
    const hello = JSON.parse(wire) as { publicKey: string; salt: string };
    const peer = await crypto.subtle.importKey("raw", remoteCodec.base64ToBytes(hello.publicKey), { name: "X25519" }, false, []);
    const secret = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: peer }, this.privateKey, 256));
    const material = await crypto.subtle.importKey("raw", secret, "HKDF", false, ["deriveKey"]);
    this.info = new TextEncoder().encode(`reasonix-remote-v1|${this.deviceId}`);
    this.key = await crypto.subtle.deriveKey(
      { name: "HKDF", hash: "SHA-256", salt: remoteCodec.base64ToBytes(hello.salt), info: this.info },
      material,
      { name: "AES-GCM", length: 256 },
      false,
      ["encrypt", "decrypt"],
    );
    await this.seal({ v: 1, type: "ready", deviceId: this.claim, features: [], session: "s-1", instance: "desktop-1" });
  }

  private async open(wire: string) {
    const envelope = JSON.parse(wire) as { nonce: string; ciphertext: string };
    const plain = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: remoteCodec.base64ToBytes(envelope.nonce), additionalData: this.info },
      this.key as CryptoKey,
      remoteCodec.base64ToBytes(envelope.ciphertext),
    );
    return JSON.parse(new TextDecoder().decode(plain)) as Record<string, unknown>;
  }

  private async seal(value: unknown) {
    const nonce = crypto.getRandomValues(new Uint8Array(12));
    const ciphertext = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce, additionalData: this.info },
      this.key as CryptoKey,
      new TextEncoder().encode(JSON.stringify(value)),
    );
    this.socket.deliver(JSON.stringify({ v: 1, nonce: remoteCodec.bytesToBase64(nonce), ciphertext: remoteCodec.bytesToBase64(new Uint8Array(ciphertext)) }));
  }
}

class FakeSocket extends EventTarget {
  static OPEN = 1;
  static CLOSED = 3;
  static readonly instances: FakeSocket[] = [];
  static desktop: (socket: FakeSocket) => FakeDesktop | null = () => null;

  readyState = 0;
  bufferedAmount = 0;
  desktop: FakeDesktop | null = null;
  readonly sent: string[] = [];

  constructor(
    readonly url: string,
    readonly protocols: string[],
  ) {
    super();
    FakeSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = FakeSocket.OPEN;
      this.desktop = FakeSocket.desktop(this);
      this.dispatchEvent(new Event("open"));
    });
  }

  send(wire: string) {
    this.sent.push(wire);
    void this.desktop?.receive(wire);
  }

  deliver(wire: string) {
    this.dispatchEvent(new MessageEvent("message", { data: wire }));
  }

  close(code = 1000, reason = "") {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    this.dispatchEvent(Object.assign(new Event("close"), { code, reason }));
  }
}

const nativeFetch = globalThis.fetch;
const nativeWebSocket = globalThis.WebSocket;
const nativeEventSource = globalThis.EventSource;

afterEach(() => {
  globalThis.fetch = nativeFetch;
  globalThis.WebSocket = nativeWebSocket;
  globalThis.EventSource = nativeEventSource;
  FakeSocket.instances.length = 0;
  FakeSocket.desktop = () => null;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function stubPairing(dev: Device, status = 200, claimedRelay = "wss://relay.test"): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input instanceof Request ? input.url : input);
    if (!url.endsWith("/v1/pair/123456")) return new Response("ok", { status: 200 });
    if (status !== 200) {
      return new Response(JSON.stringify({ error: { code: "invalid_pair_code" } }), { status });
    }
    return new Response(JSON.stringify({
      deviceId: dev.id, publicKey: dev.publicKey, relay: claimedRelay, controlTicket: "ticket-1",
    }), { status: 200, headers: { "content-type": "application/json" } });
  });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
  FakeSocket.desktop = (socket) => new FakeDesktop(dev.id, dev.privateKey, socket);
  return fetchMock;
}

function ends(): { list: RelayLinkError[]; stop: () => void } {
  const list: RelayLinkError[] = [];
  const stop = onRelayEnded((error) => list.push(error));
  return { list, stop };
}

describe("redeeming a pairing code", () => {
  it("refuses a code that is not six digits without asking the relay", async () => {
    const dev = await device("dev-1");
    const fetchMock = stubPairing(dev);

    await expect(redeemAndConnect("https://relay.test", "12345")).rejects.toMatchObject({ kind: "invalid_code" });
    await expect(redeemAndConnect("https://relay.test", "12a456")).rejects.toMatchObject({ kind: "invalid_code" });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(FakeSocket.instances).toHaveLength(0);
  });

  it("maps an unknown or expired code to invalid_code and opens no socket", async () => {
    const dev = await device("dev-1");
    const fetchMock = stubPairing(dev, 404);

    const error = await redeemAndConnect("https://relay.test", "123456").then(() => null, (e: unknown) => e);
    expect(error).toMatchObject({ kind: "invalid_code" });
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(String(fetchMock.mock.calls[0][0])).toBe("https://relay.test/v1/pair/123456");
    expect(FakeSocket.instances).toHaveLength(0);
  });

  it("says offline when the relay cannot be reached at all", async () => {
    const dev = await device("dev-1");
    stubPairing(dev);
    globalThis.fetch = vi.fn(async () => { throw new TypeError("Failed to fetch"); }) as unknown as typeof fetch;

    await expect(redeemAndConnect("https://relay.test", "123456")).rejects.toMatchObject({ kind: "offline" });
  });

  // The ticket was issued for the relay the pairing names; an address of any
  // other shape would offer it somewhere it was never issued for.
  it.each(["https://relay.test", "wss://relay.test/path", "wss://relay.test/?x=1", "wss://user:pw@relay.test", "not a url"])(
    "refuses a pairing that points the ticket at %s",
    async (named) => {
      const dev = await device("dev-1");
      stubPairing(dev, 200, named);

      await expect(redeemAndConnect("https://relay.test", "123456")).rejects.toMatchObject({ kind: "protocol" });
      expect(FakeSocket.instances).toHaveLength(0);
    },
  );

  it("refuses a ready frame from a computer the code did not name", async () => {
    const dev = await device("dev-1");
    stubPairing(dev);
    FakeSocket.desktop = (socket) => new FakeDesktop(dev.id, dev.privateKey, socket, "dev-2");

    await expect(redeemAndConnect("https://relay.test", "123456")).rejects.toMatchObject({ kind: "protocol" });
  });

  it("reads the relay origin of the configured base, trailing slashes and all", () => {
    expect(relayBase()).toBe(location.origin);
  });
});

describe("the paired channel", () => {
  it("consumes the ready frame and carries same-origin requests over the tunnel", async () => {
    const dev = await device("dev-1");
    const fetchMock = stubPairing(dev);
    const hub = await redeemAndConnect("https://relay.test/", "123456");

    expect(fetchMock.mock.calls[0][0]).toBe("https://relay.test/v1/pair/123456");
    const socket = FakeSocket.instances[0];
    expect(socket.url).toBe("wss://relay.test/v1/sessions/connect");
    expect(socket.protocols).toEqual(["reasonix.remote.v1", "reasonix.auth.ticket-1"]);
    // The ready frame was consumed as the handshake, not left queued for a request.
    expect(socket.sent).toHaveLength(1);
    expect(JSON.parse(socket.sent[0])).toMatchObject({ v: 1, type: "hello" });

    const port = hub.portFor({ id: "r1", base: "/rt/r1", root: "", name: "one" });
    const status = port.status();
    expect(socket.sent).toHaveLength(1);
    await expect(status).resolves.toEqual({ ok: true });

    // What crossed is a sealed envelope, and the desktop opens it as the request
    // this page made — under the session name and sequence the ready frame set.
    expect(socket.sent).toHaveLength(2);
    const wire = JSON.parse(socket.sent[1]) as Record<string, unknown>;
    expect(wire).toMatchObject({ v: 1, nonce: expect.any(String), ciphertext: expect.any(String) });
    expect(socket.sent[1]).not.toContain("/rt/r1/status");
    const desktop = socket.desktop as FakeDesktop;
    expect(desktop.opened[0]).toMatchObject({ v: 1, type: "desktop.request", method: "GET", path: "/rt/r1/status", session: "s-1", seq: 1 });
  });

  it("leaves requests to another origin on the page's own network", async () => {
    const dev = await device("dev-1");
    // The stub is the page's own fetch; the tunnel captured it at install time.
    const fetchMock = stubPairing(dev);
    await redeemAndConnect("https://relay.test", "123456");
    const socket = FakeSocket.instances[0];

    const answer = await globalThis.fetch("https://example.test/thing");
    expect(await answer.text()).toBe("ok");
    expect(fetchMock).toHaveBeenCalledTimes(2);
    const last = fetchMock.mock.calls[1][0] as Request | string;
    expect(String(last instanceof Request ? last.url : last)).toBe("https://example.test/thing");
    expect(socket.sent).toHaveLength(1);
  });

  it("tells the page the link is over when the device is revoked", async () => {
    const dev = await device("dev-1");
    stubPairing(dev);
    await redeemAndConnect("https://relay.test", "123456");
    const { list, stop } = ends();

    FakeSocket.instances[0].close(4403, "device revoked");

    expect(list).toHaveLength(1);
    expect(list[0]).toBeInstanceOf(RelayLinkError);
    expect(list[0]).toMatchObject({ kind: "ended", reason: "device revoked" });
    expect(relayEnded()).toBe(list[0]);
    stop();
  });

  it("tells the page the link is over when the desktop disconnects", async () => {
    const dev = await device("dev-1");
    stubPairing(dev);
    await redeemAndConnect("https://relay.test", "123456");
    const { list, stop } = ends();

    FakeSocket.instances[0].close(4004, "device disconnected");

    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ kind: "ended", reason: "device disconnected" });
    stop();
  });
});

describe("polls a stream while only the tunnel can reach it", () => {
  function stubReplay(...answers: unknown[]) {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL) =>
      new Response(JSON.stringify(answers.length > 1 ? answers.shift() : answers[0]), { status: 200 }));
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    return fetchMock;
  }

  it("starts polling after the host watermark instead of replaying restored history", async () => {
    vi.useFakeTimers();
    const fetchMock = stubReplay(
      { frames: null, complete: true, watermark: 7 },
      { frames: [{ kind: "message", seq: 8, text: "new reply" }], complete: true, watermark: 8 },
    );
    const seen: Array<Record<string, unknown>> = [];
    const source = new RemoteEventSource("/rt/one/events");
    source.onmessage = (event) => seen.push(JSON.parse(event.data) as Record<string, unknown>);

    await vi.advanceTimersByTimeAsync(0);
    // No cursor is an attach, and an attach is answered with whatever a run is
    // stopped on rather than with the stream's history.
    expect(String(fetchMock.mock.calls[0][0])).toBe("/rt/one/events/replay");
    expect(seen).toEqual([{ kind: "stream_watermark", seq: 7 }]);

    await vi.advanceTimersByTimeAsync(1500);
    expect(String(fetchMock.mock.calls[1][0])).toBe("/rt/one/events/replay?lastEventId=7");
    expect(seen.slice(1)).toEqual([{ kind: "message", seq: 8, text: "new reply" }]);

    source.close();
  });

  it("hands an attaching page the question a run is waiting on", async () => {
    vi.useFakeTimers();
    const fetchMock = stubReplay(
      { frames: [{ kind: "ask_request", ask: { id: "ask-1" } }], complete: true, watermark: 3 },
      { frames: [], complete: true, watermark: 3 },
    );
    const seen: Array<Record<string, unknown>> = [];
    const source = new RemoteEventSource("/rt/one/events");
    source.onmessage = (event) => seen.push(JSON.parse(event.data) as Record<string, unknown>);

    await vi.advanceTimersByTimeAsync(0);
    // What a run is blocked on is not in the ordinary history a client resumes
    // past, so a page that read only the log would draw the run as stopped with
    // nothing to answer.
    expect(seen).toEqual([{ kind: "ask_request", ask: { id: "ask-1" } }]);

    await vi.advanceTimersByTimeAsync(200);
    expect(String(fetchMock.mock.calls[1][0])).toBe("/rt/one/events/replay?lastEventId=3");
    source.close();
  });

  it("resumes from the cursor its URL carries", async () => {
    vi.useFakeTimers();
    const fetchMock = stubReplay({ frames: [{ kind: "message", seq: 6, text: "missed while away" }], complete: true, watermark: 6 });
    const seen: Array<Record<string, unknown>> = [];
    const source = new RemoteEventSource("/rt/one/events?lastEventId=5");
    source.onmessage = (event) => seen.push(JSON.parse(event.data) as Record<string, unknown>);

    await vi.advanceTimersByTimeAsync(0);
    source.close();
    expect(String(fetchMock.mock.calls[0][0])).toBe("/rt/one/events/replay?lastEventId=5");
    expect(seen).toEqual([{ kind: "message", seq: 6, text: "missed while away" }]);
  });

  it("says so when the log no longer reaches the cursor", async () => {
    vi.useFakeTimers();
    stubReplay({ frames: [{ kind: "message", seq: 40, text: "oldest kept" }], complete: false, watermark: 40 });
    const seen: Array<Record<string, unknown>> = [];
    const source = new RemoteEventSource("/rt/one/events?lastEventId=5");
    source.onmessage = (event) => seen.push(JSON.parse(event.data) as Record<string, unknown>);

    await vi.advanceTimersByTimeAsync(0);
    source.close();
    expect(seen).toEqual([
      { kind: "stream_gap", seq: 40 },
      { kind: "message", seq: 40, text: "oldest kept" },
    ]);
  });

  it("keeps an idle pane's stream under the silence watchdog", async () => {
    vi.useFakeTimers();
    const fetchMock = stubReplay({ frames: null, complete: true, watermark: 7 });
    globalThis.EventSource = RemoteEventSource as unknown as typeof EventSource;
    let opened = 0;
    const stop = openLiveStream(() => `/rt/one/events${opened++ ? "?lastEventId=7" : ""}`, () => {});

    await vi.advanceTimersByTimeAsync(STREAM_SILENCE_MS * 3);
    stop();
    expect(opened).toBe(1);
    // Every read goes to the replay route: the attach carries no cursor, and
    // every later read resumes from the watermark.
    for (const [input] of fetchMock.mock.calls) expect(String(input)).toMatch(/^\/rt\/one\/events\/replay(\?|$)/);
  });
});
