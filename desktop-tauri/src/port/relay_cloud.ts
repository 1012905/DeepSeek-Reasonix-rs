import { SseHub, type HubPort } from "./hub";
import { failureView, retryAfterSeconds, type FailureDetail } from "./cloud_failure";
import {
  encryptedChannel,
  HandshakeTimeout,
  nextMessage,
  RemoteLink,
  RemoteLinkError,
  SocketRefused,
  waitForSocketOpen,
  type RemoteConnection,
  type RemoteEnd,
} from "./cloud_link";

// The subprotocol a hosted page asks for. The ticket rides in the second one
// because a browser cannot set an Authorization header on a WebSocket.
const PROTOCOL = "reasonix.remote.v1";
const AUTH_PREFIX = "reasonix.auth.";
const PAIR_CODE = /^\d{6}$/;

/** The origin the pairing page and the relay share: the build's own relay when
 *  one is configured, otherwise the origin this page was served from. */
export function relayBase(): string {
  return (import.meta.env.VITE_RELAY_URL || location.origin).replace(/\/+$/, "");
}

// Installed by redeemAndConnect. Pairing must not travel back through the
// tunnel this module already installed: a re-pair happens after that link ended.
let tunneled: { patched: typeof fetch; native: typeof fetch } | null = null;

async function plainFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const call = tunneled !== null && globalThis.fetch === tunneled.patched ? tunneled.native : globalThis.fetch;
  return call(input, init);
}

export class RelayLinkError extends Error {
  constructor(
    readonly kind: "invalid_code" | "offline" | "protocol" | "ended",
    readonly reason: string,
    readonly failure?: FailureDetail,
  ) {
    super(reason);
    this.name = "RelayLinkError";
  }
}

interface Pairing {
  deviceId: string;
  publicKey: string;
  relay: string;
  controlTicket: string;
}

let ended: RelayLinkError | null = null;
const endedListeners = new Set<(error: RelayLinkError) => void>();

function announce(end: RemoteEnd) {
  ended = new RelayLinkError("ended", end.reason, end.failure);
  for (const listener of endedListeners) listener(ended);
}

/** Why this page's link is over, or null while it still stands. */
export const relayEnded = () => ended;

// The link cannot be resumed with the ticket this page holds: the desktop left,
// the device was revoked, or a newer connection took the desktop's place.
export function onRelayEnded(listener: (error: RelayLinkError) => void) {
  endedListeners.add(listener);
  return () => { endedListeners.delete(listener); };
}

// Rate limits and outages are the same "try again shortly" answer to the page;
// the detail says which one it was.
function later(detail: FailureDetail): RelayLinkError {
  return new RelayLinkError("offline", failureView(detail).body, detail);
}

function offline(): RelayLinkError {
  return later({ code: navigator.onLine === false ? "offline" : "unreachable" });
}

async function refusal(answer: Response): Promise<RelayLinkError> {
  if (answer.status === 429) {
    return later({ code: "rate_limited", retryAfterS: retryAfterSeconds(answer.headers.get("retry-after")) });
  }
  if (answer.status >= 500) return later({ code: "unreachable" });
  return new RelayLinkError("protocol", "The relay would not hand out a pairing for this code.");
}

// The pairing names the relay the ticket was issued for; anything but a bare
// websocket origin is refused so a ticket is never offered elsewhere.
function relayOrigin(named: unknown): string | null {
  if (typeof named !== "string") return null;
  let url: URL;
  try {
    url = new URL(named);
  } catch {
    return null;
  }
  if (url.protocol !== "wss:" && url.protocol !== "ws:") return null;
  if (url.username || url.password || url.search || url.hash) return null;
  if (url.pathname !== "/" && url.pathname !== "") return null;
  return `${url.protocol}//${url.host}`;
}

async function redeem(base: string, code: string): Promise<Pairing> {
  if (!PAIR_CODE.test(code)) {
    throw new RelayLinkError("invalid_code", "A pairing code is six digits.");
  }
  let answer: Response;
  try {
    answer = await plainFetch(`${base}/v1/pair/${code}`, { cache: "no-store" });
  } catch {
    throw offline();
  }
  if (answer.status === 404 || answer.status === 410) {
    throw new RelayLinkError("invalid_code", "That code is unknown or has expired.");
  }
  if (!answer.ok) throw await refusal(answer);
  const body = (await answer.json().catch(() => null)) as Partial<Pairing> | null;
  if (
    typeof body?.deviceId !== "string" || body.deviceId === "" ||
    typeof body.publicKey !== "string" || body.publicKey === "" ||
    typeof body.controlTicket !== "string" || body.controlTicket === ""
  ) {
    throw new RelayLinkError("protocol", "The relay's answer to this code was not a pairing.");
  }
  const relay = relayOrigin(body.relay);
  if (!relay) throw new RelayLinkError("protocol", "The pairing named a relay address this page cannot speak to.");
  return { deviceId: body.deviceId, publicKey: body.publicKey, relay, controlTicket: body.controlTicket };
}

async function connectTicket(pairing: Pairing): Promise<RemoteConnection> {
  const socket = new WebSocket(`${pairing.relay}/v1/sessions/connect`, [PROTOCOL, `${AUTH_PREFIX}${pairing.controlTicket}`]);
  try {
    await waitForSocketOpen(socket);
    const readyWire = nextMessage(socket);
    const channel = await encryptedChannel({ id: pairing.deviceId, publicKey: pairing.publicKey }, socket);
    const ready = await channel.open(await readyWire);
    if (ready.type !== "ready" || ready.deviceId !== pairing.deviceId) {
      throw new RelayLinkError("protocol", "The computer on the other end is not the one this code named.");
    }
    const features = Array.isArray(ready.features)
      ? ready.features.filter((item): item is string => typeof item === "string")
      : [];
    if (typeof ready.session === "string" && ready.session) channel.bind(ready.session);
    const instance = typeof ready.instance === "string" ? ready.instance : "";
    return {
      socket, channel, features, instance,
      listen(onMessage, onClose) {
        socket.addEventListener("message", (event) => onMessage(String(event.data)));
        socket.addEventListener("close", (event) => onClose(event.code, event.reason));
      },
    };
  } catch (error) {
    try { socket.close(1000, "Handshake failed"); } catch { /* never opened */ }
    if (error instanceof RelayLinkError) throw error;
    if (error instanceof SocketRefused) throw offline();
    if (error instanceof HandshakeTimeout) throw new RelayLinkError("offline", error.reason, { code: "unknown" });
    throw new RelayLinkError("protocol", "The handshake with the remote Studio did not complete.");
  }
}

// A refused ticket, or a desktop that answers with another identity, cannot be
// reached again with the ticket this page holds; a network failure can.
function asDialError(error: unknown): unknown {
  if (!(error instanceof RelayLinkError)) return error;
  return new RemoteLinkError(error.kind === "protocol" ? "ended" : "transient", error.reason, error.failure);
}

// Stands in for EventSource over the relay by polling /events/replay. A cursor
// the URL carries is where a replacement stream resumes. Frames the SSE
// transport writes on its own (the watermark, a gap) are said here too, or the
// stream reads as silent to whoever watches it.
export class RemoteEventSource {
  onmessage: ((event: MessageEvent<string>) => void) | null = null;
  private closed = false;
  private after: number | null;
  private readonly replay: string;
  // Only the main event stream carries connection-local state to hand back —
  // the pending prompt — so only its attach asks for nothing. The tree stream
  // reads the tree, not history, so its attach asks past the watermark.
  private readonly attachQuery: string;

  constructor(url: string) {
    const target = new URL(url, location.origin);
    const cursor = Number(target.searchParams.get("lastEventId") ?? "");
    this.after = target.searchParams.has("lastEventId") && Number.isSafeInteger(cursor) && cursor >= 0 ? cursor : null;
    this.replay = `${target.pathname}/replay`;
    this.attachQuery = target.pathname.endsWith("/tree/events") ? `?lastEventId=${Number.MAX_SAFE_INTEGER}` : "";
    queueMicrotask(() => void this.poll());
  }

  close() {
    this.closed = true;
  }

  private say(frame: Record<string, unknown>) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(frame) }));
  }

  private async poll() {
    while (!this.closed) {
      let active = false;
      try {
        // No cursor yet is an attach; every later read resumes from where it
        // stands, so the page never draws history it already has.
        const query = this.after !== null ? `?lastEventId=${this.after}` : this.attachQuery;
        const response = await fetch(`${this.replay}${query}`, { credentials: "same-origin" });
        if (response.ok && !this.closed) {
          const body = await response.json() as { frames?: Array<Record<string, unknown>> | null; complete?: boolean; watermark?: number };
          const watermark = typeof body.watermark === "number" ? body.watermark : null;
          const frames = body.frames ?? [];
          active = frames.length > 0;
          if (this.after === null) this.after = watermark ?? 0;
          else if (body.complete === false) {
            const from = typeof frames[0]?.seq === "number" ? frames[0].seq : watermark ?? this.after;
            this.say({ kind: "stream_gap", seq: from });
          }
          for (const frame of frames) {
            const seq = typeof frame.seq === "number" ? frame.seq : 0;
            if (seq > this.after) this.after = seq;
            this.say(frame);
          }
          if (!active && watermark !== null) this.say({ kind: "stream_watermark", seq: watermark });
        }
      } catch {
        // The next poll retries while the encrypted session remains open.
      }
      await new Promise((resolve) => setTimeout(resolve, active ? 200 : 1500));
    }
  }
}

/** Trades a pairing code for one ticket, opens the encrypted channel, and
 *  returns the hub every request from then on travels through. The ticket is
 *  kept for the page's lifetime: reconnects rejoin with it, never a new code. */
export async function redeemAndConnect(base: string, code: string): Promise<HubPort> {
  ended = null;
  const pairing = await redeem(base.replace(/\/+$/, ""), code.trim());
  const first = await connectTicket(pairing);
  const nativeFetch = globalThis.fetch.bind(globalThis);
  const link = new RemoteLink(first, () => connectTicket(pairing).catch((e: unknown) => { throw asDialError(e); }), announce);
  const patched = ((input: RequestInfo | URL, init?: RequestInit) => {
    // A relative input resolves against this page, as the Fetch spec says; a
    // Request built from one alone does not resolve it in every environment.
    const request = input instanceof Request ? new Request(input, init) : new Request(new URL(String(input), location.href), init);
    return new URL(request.url).origin === location.origin ? link.fetch(request) : nativeFetch(request);
  }) as typeof fetch;
  tunneled = { patched, native: nativeFetch };
  globalThis.fetch = patched;
  globalThis.EventSource = RemoteEventSource as unknown as typeof EventSource;
  // RemoteEventSource polls one stream, and the multiplexed stream also needs an
  // open frame and per-pane subscriptions it cannot carry, so panes read their own.
  return new SseHub({ mux: false });
}
