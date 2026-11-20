import type { FailureDetail, RelayFailure } from "./cloud_failure";

// Relay close codes, decided by the relay and read here by number alone.
export const CLOSE_IDLE = 4408;
export const CLOSE_REPLACED = 4401;
export const CLOSE_REVOKED = 4403;
export const CLOSE_DISCONNECTED_BY_DEVICE = 4004;
const CLOSE_RESYNC = 4000;

export type CloseAction = "reconnect" | "end";

export function closeAction(code: number): CloseAction {
  // A replaced desktop and a revoked device leave nothing to reconnect to.
  if (code === CLOSE_REPLACED || code === CLOSE_REVOKED || code === CLOSE_DISCONNECTED_BY_DEVICE) return "end";
  // Protocol violations would repeat on a new connection.
  if (code === 1003 || code === 1008 || code === 1009) return "end";
  return "reconnect";
}

export interface RemoteEnd {
  kind: "ended";
  reason: string;
  failure?: FailureDetail;
}

// Why a (re)connection attempt failed: "transient" is worth another attempt,
// "ended" is not.
export class RemoteLinkError extends Error {
  constructor(
    readonly kind: "ended" | "transient",
    readonly reason: string,
    readonly failure?: FailureDetail,
  ) {
    super(reason);
  }
}

export interface RemoteChannel {
  seal(value: unknown): Promise<string>;
  open(payload: string): Promise<Record<string, unknown>>;
}

export interface RemoteConnection {
  socket: WebSocket;
  channel: RemoteChannel;
  features: string[];
  instance: string;
  listen(onMessage: (data: string) => void, onClose: (code: number, reason: string) => void): void;
}

interface Pending {
  id: string;
  method: string;
  // How many commands make up this request and how to build one: a request
  // too large for a message crosses as numbered parts, built as they are sent.
  count: number;
  command: (index: number) => Record<string, unknown>;
  sentTo: RemoteConnection | null;
  chunks: Map<number, string>;
  status?: number;
  contentType?: string;
  etag?: string;
  timer?: ReturnType<typeof setTimeout>;
  // When this request last made progress: sent whole, or one more chunk of its answer.
  progressAt?: number;
  resolve: (value: Response) => void;
  reject: (reason: unknown) => void;
}

export interface RemoteLinkOptions {
  requestTimeoutMs: number;
  retryDelaysMs: number[];
}

const MAX_RETRY_AFTER_MS = 2 * 60 * 1000;

// A request can wait behind others this page knows nothing about, so it is
// waited for while the link delivers their answers, up to this long silent.
const MAX_QUEUED_MS = 5 * 60 * 1000;

// The relay refuses a message over 64 KiB, taking every other request on the
// link down with it. A request that would seal past this is refused here.
const WIRE_BUDGET = 62 * 1024;
// What sealing adds around the command: the session name and sequence it is
// stamped with, the AES-GCM tag, and the envelope's own keys.
const SEAL_OVERHEAD = 16 + 96 + 120;

/** The most body bytes a request to this path can carry over the link. */
export function maxRequestBody(path: string): number {
  const plain = Math.floor(((WIRE_BUDGET - SEAL_OVERHEAD) * 3) / 4);
  const fixed = 160 + path.length;
  return Math.max(0, Math.floor(((plain - fixed) * 3) / 4));
}

// A body larger than one message crosses as parts of this many bytes, up to
// the most the desktop will hold. Both numbers are the desktop's own limits.
const UPLOAD_PART = 24 * 1024;
const MAX_UPLOAD = 16 * 1024 * 1024;
const BUFFER_HIGH = 1024 * 1024;

const TOO_LARGE_16MB = "That file is too large to send over the relay (limit 16 MB).";
const NO_RESPONSE = "Remote Studio did not respond.";
const LINK_ENDED = "The remote connection ended.";

async function requestBytes(request: Request): Promise<Uint8Array> {
  const uploadSignal: AbortSignal = request.signal;
  uploadSignal.throwIfAborted();
  if (!request.body) return new Uint8Array();
  const reader = request.body.getReader();
  const abort = () => { void reader.cancel(uploadSignal.reason); };
  uploadSignal.addEventListener("abort", abort, { once: true });
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      uploadSignal.throwIfAborted();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_UPLOAD) {
        await reader.cancel();
        throw new Error(TOO_LARGE_16MB);
      }
      chunks.push(value);
    }
  } finally {
    uploadSignal.removeEventListener("abort", abort);
    reader.releaseLock();
  }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.length; }
  return body;
}

const DEFAULTS: RemoteLinkOptions = {
  requestTimeoutMs: 30_000,
  retryDelaysMs: [500, 1_000, 2_000, 4_000, 8_000, 15_000],
};

export function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function base64ToBytes(value: string) {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - value.length % 4) % 4);
  return Uint8Array.from(atob(padded), (char) => char.charCodeAt(0));
}

export function concatChunks(chunks: Map<number, string>) {
  const parts = [...chunks.entries()].sort(([a], [b]) => a - b).map(([, value]) => base64ToBytes(value));
  const joined = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.length;
  }
  return joined;
}

type Bytes = Uint8Array<ArrayBuffer>;

export const HANDSHAKE_TIMEOUT_MS = 30_000;

// Browsers older than Chromium 133, Firefox 130 and Safari 17 have no X25519 in
// WebCrypto and say so with a NotSupportedError. Those take the audited
// software curve, loaded only then; the rest of the channel stays WebCrypto.
export async function agreement(devicePublic: Bytes): Promise<{ secret: Bytes; publicKey: Bytes }> {
  try {
    const pair = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]) as CryptoKeyPair;
    const deviceKey = await crypto.subtle.importKey("raw", devicePublic, { name: "X25519" }, false, []);
    const secret = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: deviceKey }, pair.privateKey, 256));
    return { secret, publicKey: new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey)) };
  } catch (error) {
    if (!(error instanceof DOMException && error.name === "NotSupportedError")) throw error;
    return softwareAgreement(devicePublic);
  }
}

async function softwareAgreement(devicePublic: Bytes): Promise<{ secret: Bytes; publicKey: Bytes }> {
  const { x25519 } = await import("@noble/curves/ed25519.js").catch(() => {
    throw new RemoteLinkError("transient", "Could not load the encryption this page needs.");
  });
  const keys = x25519.keygen();
  return { secret: new Uint8Array(x25519.getSharedSecret(keys.secretKey, devicePublic)), publicKey: new Uint8Array(keys.publicKey) };
}

export interface SealedChannel {
  bind(session: string): void;
  seal(value: unknown): Promise<string>;
  open(payload: string): Promise<Record<string, unknown>>;
}

// The encryption this page and the desktop share. The key is X25519 against the
// device's public key, stretched with the device id as the only purpose, so a
// relay cannot read, replay or reorder what crosses it.
export async function encryptedChannel(
  device: { id: string; publicKey: string },
  socket: WebSocket,
): Promise<SealedChannel> {
  const { secret, publicKey } = await agreement(base64ToBytes(device.publicKey) as Bytes);
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const material = await crypto.subtle.importKey("raw", secret, "HKDF", false, ["deriveKey"]);
  const info = new TextEncoder().encode(`reasonix-remote-v1|${device.id}`);
  const key = await crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt, info },
    material,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  socket.send(JSON.stringify({
    v: 1, type: "hello", publicKey: bytesToBase64(publicKey), salt: bytesToBase64(salt),
  }));
  // Once the desktop names this session, every command carries that name and
  // an increasing number, so a relay cannot replay a command into a later
  // session or twice into this one.
  let bound: string | null = null;
  let seq = 0;
  return {
    bind(session: string) {
      bound = session;
    },
    async seal(value: unknown) {
      const nonce = crypto.getRandomValues(new Uint8Array(12));
      const stamped = bound && value && typeof value === "object"
        ? { ...(value as Record<string, unknown>), session: bound, seq: ++seq }
        : value;
      const plain = new TextEncoder().encode(JSON.stringify(stamped));
      const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: info }, key, plain);
      return JSON.stringify({ v: 1, nonce: bytesToBase64(nonce), ciphertext: bytesToBase64(new Uint8Array(ciphertext)) });
    },
    async open(payload: string) {
      const envelope = JSON.parse(payload) as { v: number; nonce: string; ciphertext: string };
      if (envelope.v !== 1) throw new Error("Unsupported remote encryption version.");
      const plain = await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: base64ToBytes(envelope.nonce), additionalData: info },
        key,
        base64ToBytes(envelope.ciphertext),
      );
      return JSON.parse(new TextDecoder().decode(plain)) as Record<string, unknown>;
    },
  };
}

// The socket errored before it opened: the browser hides the status, so the
// caller has to find out why.
export class SocketRefused extends Error {}

// The relay accepted the socket but nothing spoke in time. Unlike a refused
// upgrade this is worth another attempt.
export class HandshakeTimeout extends Error {
  constructor(readonly reason: string) {
    super(reason);
  }
}

export function waitForSocketOpen(socket: WebSocket, timeout = HANDSHAKE_TIMEOUT_MS): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      socket.removeEventListener("open", opened);
      socket.removeEventListener("error", failed);
    };
    const opened = () => { cleanup(); resolve(); };
    const failed = () => {
      cleanup();
      reject(new SocketRefused());
    };
    const timer = setTimeout(() => {
      cleanup();
      try { socket.close(); } catch { /* The browser may reject closing a socket that never left CONNECTING. */ }
      reject(new HandshakeTimeout("Connecting to remote Studio timed out."));
    }, timeout);
    socket.addEventListener("open", opened);
    socket.addEventListener("error", failed);
  });
}

export function nextMessage(socket: WebSocket, timeout = HANDSHAKE_TIMEOUT_MS): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new HandshakeTimeout(NO_RESPONSE));
    }, timeout);
    const cleanup = () => {
      clearTimeout(timer);
      socket.removeEventListener("message", receive);
      socket.removeEventListener("close", closed);
    };
    const receive = (event: MessageEvent) => { cleanup(); resolve(String(event.data)); };
    const closed = () => { cleanup(); reject(new Error(LINK_ENDED)); };
    socket.addEventListener("message", receive);
    socket.addEventListener("close", closed);
  });
}

// Carries desktop requests over the relay and keeps doing so across relay
// reconnects. A request that never left, or one the desktop can safely see
// twice, is resent on the next connection under the same id; a state-changing
// one already sent to a desktop that cannot recognise a resend is failed
// instead, so it never runs twice.
export class RemoteLink {
  private current: RemoteConnection | null = null;
  private readonly pending = new Map<string, Pending>();
  private reconnecting = false;
  private ended = false;
  private lastHeardAt = Date.now();
  private lastCloseCode = 0;
  private sending: Promise<void> = Promise.resolve();
  private readonly options: RemoteLinkOptions;

  constructor(
    first: RemoteConnection,
    private readonly dial: () => Promise<RemoteConnection>,
    private readonly onEnd: (end: RemoteEnd) => void,
    options: Partial<RemoteLinkOptions> = {},
  ) {
    this.options = { ...DEFAULTS, ...options };
    this.attach(first);
  }

  async fetch(request: Request): Promise<Response> {
    if (this.ended) throw new Error(LINK_ENDED);
    const id = crypto.randomUUID();
    const body = await requestBytes(request);
    const path = request.url.slice(location.origin.length);
    const room = maxRequestBody(path);
    const method = request.method;
    let count = 1;
    let command: Pending["command"] = () => ({ v: 1, type: "desktop.request", id, method, path, body: bytesToBase64(body) });
    if (body.length > room) {
      if (!this.current?.features.includes("upload")) {
        throw new Error(`That request is too large to send over the relay (limit ${Math.floor(room / 1024)} KB).`);
      }
      if (body.length > MAX_UPLOAD) {
        throw new Error(TOO_LARGE_16MB);
      }
      const parts = Math.ceil(body.length / UPLOAD_PART);
      count = parts;
      command = (index) => ({
        v: 1, type: "desktop.request.part", id, index, total: parts,
        ...(index === 0 ? { method, path } : {}),
        body: bytesToBase64(body.subarray(index * UPLOAD_PART, (index + 1) * UPLOAD_PART)),
      });
    }
    return new Promise<Response>((resolve, reject) => {
      const entry: Pending = {
        id, method: method.toUpperCase(), count, command,
        sentTo: null, chunks: new Map(), resolve, reject,
      };
      this.pending.set(id, entry);
      void this.send(entry);
    });
  }

  private attach(connection: RemoteConnection) {
    this.current = connection;
    this.lastHeardAt = Date.now();
    connection.listen((data) => {
      if (this.current !== connection) return;
      this.lastHeardAt = Date.now();
      void this.receive(connection, data);
    }, (code, reason) => {
      if (this.current === connection) this.lost(code, reason);
    });
  }

  // Commands are numbered as they are sealed, so they leave in sealing order.
  private send(entry: Pending): Promise<void> {
    this.sending = this.sending.then(() => this.sendNow(entry), () => this.sendNow(entry));
    return this.sending;
  }

  private async sendNow(entry: Pending) {
    const connection = this.current;
    if (!connection || connection.socket.readyState !== WebSocket.OPEN || !this.pending.has(entry.id)) return;
    const live = () => this.current === connection && connection.socket.readyState === WebSocket.OPEN && this.pending.has(entry.id);
    clearTimeout(entry.timer);
    for (let index = 0; index < entry.count; index++) {
      const wire = await connection.channel.seal(entry.command(index));
      if (!live()) return;
      if (index === 0) {
        entry.chunks.clear();
        entry.sentTo = connection;
      }
      connection.socket.send(wire);
      // A large upload would otherwise queue all of itself in the socket at once.
      while (connection.socket.bufferedAmount > BUFFER_HIGH && live()) await new Promise((resolve) => setTimeout(resolve, 20));
      if (!live()) return;
    }
    this.arm(entry);
  }

  private arm(entry: Pending) {
    clearTimeout(entry.timer);
    entry.progressAt = Date.now();
    entry.timer = setTimeout(() => this.timedOut(entry), this.options.requestTimeoutMs);
  }

  // A desktop that has said nothing at all for a whole request timeout has lost
  // this connection's encrypted session; one that is merely slow has not.
  private timedOut(entry: Pending) {
    if (!this.pending.has(entry.id)) return;
    if (this.current && Date.now() - this.lastHeardAt >= this.options.requestTimeoutMs) {
      const stale = this.current;
      this.lost(CLOSE_RESYNC, NO_RESPONSE);
      try { stale.socket.close(CLOSE_RESYNC, "Resync"); } catch { /* already closing */ }
      return;
    }
    if (Date.now() - (entry.progressAt ?? 0) < MAX_QUEUED_MS) {
      entry.timer = setTimeout(() => this.timedOut(entry), this.options.requestTimeoutMs);
      return;
    }
    this.pending.delete(entry.id);
    entry.reject(new Error(NO_RESPONSE));
  }

  private lost(code: number, reason: string) {
    if (this.ended) return;
    this.current = null;
    this.lastCloseCode = code;
    if (closeAction(code) !== "reconnect") {
      this.finish({ kind: "ended", reason });
      return;
    }
    for (const entry of this.pending.values()) clearTimeout(entry.timer);
    this.settleUnanswered((sentTo) => sentTo.features.includes("replay"));
    if (!this.reconnecting) void this.reconnect();
  }

  private async reconnect() {
    this.reconnecting = true;
    let failure: FailureDetail | undefined;
    let notBefore = 0;
    try {
      for (const scheduled of this.options.retryDelaysMs) {
        await new Promise((resolve) => setTimeout(resolve, Math.max(scheduled, notBefore)));
        if (this.ended) return;
        try {
          const connection = await this.dial();
          if (this.ended) {
            connection.socket.close(1000, "Ended");
            return;
          }
          this.attach(connection);
          this.settleUnanswered((sentTo) =>
            sentTo.features.includes("replay") && sentTo.instance !== "" && sentTo.instance === connection.instance);
          for (const entry of this.pending.values()) void this.send(entry);
          return;
        } catch (error) {
          if (error instanceof RemoteLinkError && error.kind !== "transient") {
            this.finish({ kind: "ended", reason: error.reason });
            return;
          }
          failure = error instanceof RemoteLinkError ? error.failure : undefined;
          notBefore = Math.min((failure?.retryAfterS ?? 0) * 1000, MAX_RETRY_AFTER_MS);
        }
      }
      failure ??= this.lastCloseCode === CLOSE_IDLE ? { code: "idle" satisfies RelayFailure } : undefined;
      this.finish({ kind: "ended", reason: NO_RESPONSE, ...(failure ? { failure } : {}) });
    } finally {
      this.reconnecting = false;
    }
  }

  // A state change already sent is resent only where it cannot run twice: to
  // the same desktop process, which answers a resend from its replay cache.
  private settleUnanswered(canResend: (sentTo: RemoteConnection) => boolean) {
    for (const entry of [...this.pending.values()]) {
      const idempotent = entry.method === "GET" || entry.method === "HEAD";
      if (entry.sentTo && !idempotent && !canResend(entry.sentTo)) {
        this.pending.delete(entry.id);
        entry.reject(new Error("The connection dropped before this action was confirmed."));
      }
    }
  }

  private finish(end: RemoteEnd) {
    if (this.ended) return;
    this.ended = true;
    const error = new Error(end.reason || LINK_ENDED);
    for (const entry of this.pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.pending.clear();
    this.onEnd(end);
  }

  private async receive(connection: RemoteConnection, wire: string) {
    let message: Record<string, unknown>;
    try {
      message = await connection.channel.open(wire);
    } catch {
      return;
    }
    const id = typeof message.id === "string" ? message.id : "";
    const pending = this.pending.get(id);
    if (!pending || this.current !== connection) return;
    if (message.type === "error") {
      clearTimeout(pending.timer);
      this.pending.delete(id);
      pending.reject(new Error(typeof message.error === "string" ? message.error : "Remote Studio refused this request."));
      return;
    }
    if (message.type !== "desktop.response" || typeof message.index !== "number" || typeof message.body !== "string") return;
    pending.status = typeof message.status === "number" ? message.status : pending.status;
    pending.contentType = typeof message.contentType === "string" ? message.contentType : pending.contentType;
    pending.etag = typeof message.etag === "string" ? message.etag : pending.etag;
    pending.chunks.set(message.index, message.body);
    if (message.done !== true) {
      this.arm(pending);
      return;
    }
    clearTimeout(pending.timer);
    this.pending.delete(id);
    const headers = new Headers();
    if (pending.contentType) headers.set("content-type", pending.contentType);
    if (pending.etag) headers.set("etag", pending.etag);
    const status = pending.status ?? 500;
    const bytes = concatChunks(pending.chunks);
    pending.resolve(new Response(status === 204 || status === 304 ? null : bytes, { status, headers }));
  }
}

export const remoteCodec = {
  bytesToBase64, base64ToBytes, concatChunks, agreement, encryptedChannel,
  waitForSocketOpen, nextMessage, handshakeTimeoutMS: HANDSHAKE_TIMEOUT_MS,
};
