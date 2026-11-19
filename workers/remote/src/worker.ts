import { RELAY_CONNECTION_HEADER, RELAY_ROLE_HEADER, RelayRoom, REMOTE_PROTOCOL, type SocketRole } from "./relay";

export { RelayRoom };

export interface Env {
  PAIRING: KVNamespace;
  RELAY: DurableObjectNamespace;
}

const PAIR_CODE_TTL_SECONDS = 300;
const TICKET_TTL_SECONDS = 600;
const MAX_PUBLIC_KEY_LENGTH = 4096;
const MAX_CAPABILITIES = 64;
const MAX_CAPABILITY_LENGTH = 64;
const AUTH_PROTOCOL_PREFIX = "reasonix.auth.";

const DEVICE_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)$/;
const DEVICE_PAIR_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/pair$/;
const DEVICE_ROTATE_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/rotate$/;
const DEVICE_CONNECT_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/connect$/;
const CONTROLLER_CONNECT_ROUTE = /^\/v1\/sessions\/connect$/;
const PAIR_CODE_ROUTE = /^\/v1\/pair\/(\d{6})$/;

interface DeviceRecord {
  publicKey: string;
  capabilities: string[];
  credentialHash: string;
  createdAt: number;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const { pathname } = new URL(request.url);

    if (request.method === "POST" && pathname === "/v1/devices") {
      return registerDevice(request, env);
    }

    const device = DEVICE_ROUTE.exec(pathname);
    if (request.method === "DELETE" && device !== null) {
      return revokeDevice(request, env, device[1]!);
    }

    const rotate = DEVICE_ROTATE_ROUTE.exec(pathname);
    if (request.method === "POST" && rotate !== null) {
      return rotateDevice(request, env, rotate[1]!);
    }

    const pair = DEVICE_PAIR_ROUTE.exec(pathname);
    if (request.method === "POST" && pair !== null) {
      return requestPairCode(request, env, pair[1]!);
    }

    const code = PAIR_CODE_ROUTE.exec(pathname);
    if (request.method === "GET" && code !== null) {
      return redeemPairCode(request, env, code[1]!);
    }

    const connect = DEVICE_CONNECT_ROUTE.exec(pathname);
    if (request.method === "GET" && connect !== null) {
      return forwardSocket(request, env, connect[1]!, "desktop");
    }

    // A browser cannot set an Authorization header on a WebSocket, so the
    // ticket rides in the subprotocol list the page already names.
    if (request.method === "GET" && CONTROLLER_CONNECT_ROUTE.test(pathname)) {
      return connectController(request, env);
    }

    return error(404, "not_found", "No route matches this request.");
  },
} satisfies ExportedHandler<Env>;

async function registerDevice(request: Request, env: Env): Promise<Response> {
  const body = await readJsonObject(request);
  if (body === null) {
    return error(400, "invalid_body", "Body must be a JSON object.");
  }

  const publicKey = body["publicKey"];
  const capabilities = body["capabilities"];
  if (
    typeof publicKey !== "string" ||
    publicKey.length === 0 ||
    publicKey.length > MAX_PUBLIC_KEY_LENGTH
  ) {
    return error(400, "invalid_public_key", "publicKey must be a non-empty string.");
  }
  if (!isCapabilityList(capabilities)) {
    return error(400, "invalid_capabilities", "capabilities must be an array of strings.");
  }

  const deviceId = randomToken(16);
  const deviceCredential = randomToken(32);
  const record: DeviceRecord = {
    publicKey,
    capabilities,
    credentialHash: await sha256Hex(deviceCredential),
    createdAt: Date.now(),
  };
  await env.PAIRING.put(deviceKey(deviceId), JSON.stringify(record));
  return json({ deviceId, deviceCredential }, 201);
}

async function requestPairCode(request: Request, env: Env, deviceId: string): Promise<Response> {
  if ((await authenticateDevice(request, env, deviceId)) === null) {
    return error(401, "unauthorized", "A valid device credential is required.");
  }

  for (let attempt = 0; attempt < 5; attempt += 1) {
    const code = randomPairCode();
    if ((await env.PAIRING.get(pairKey(code))) !== null) {
      continue;
    }
    await env.PAIRING.put(pairKey(code), deviceId, { expirationTtl: PAIR_CODE_TTL_SECONDS });
    return json({ code, expiresIn: PAIR_CODE_TTL_SECONDS });
  }
  return error(503, "pairing_unavailable", "Could not allocate a pairing code.");
}

async function redeemPairCode(request: Request, env: Env, code: string): Promise<Response> {
  const deviceId = await env.PAIRING.get(pairKey(code));
  if (deviceId === null) {
    return error(404, "invalid_pair_code", "Pairing code is unknown or expired.");
  }
  await env.PAIRING.delete(pairKey(code));

  const record = await loadDevice(env, deviceId);
  if (record === null) {
    return error(404, "unknown_device", "The paired device no longer exists.");
  }

  const controlTicket = mintControlTicket(deviceId);
  await env.PAIRING.put(ticketKey(await sha256Hex(controlTicket)), deviceId, {
    expirationTtl: TICKET_TTL_SECONDS,
  });
  return json({
    deviceId,
    publicKey: record.publicKey,
    relay: relayOrigin(request),
    controlTicket,
  });
}

async function rotateDevice(request: Request, env: Env, deviceId: string): Promise<Response> {
  const record = await authenticateDevice(request, env, deviceId);
  if (record === null) {
    return error(401, "unauthorized", "A valid device credential is required.");
  }
  const deviceCredential = randomToken(32);
  await env.PAIRING.put(
    deviceKey(deviceId),
    JSON.stringify({ ...record, credentialHash: await sha256Hex(deviceCredential) }),
  );
  return json({ deviceId, deviceCredential });
}

async function revokeDevice(request: Request, env: Env, deviceId: string): Promise<Response> {
  if ((await authenticateDevice(request, env, deviceId)) === null) {
    return error(401, "unauthorized", "A valid device credential is required.");
  }
  await env.PAIRING.delete(deviceKey(deviceId));
  const stub = env.RELAY.get(env.RELAY.idFromName(deviceId));
  await stub.fetch(new Request(new URL("/revoke", request.url), { method: "POST" }));
  return new Response(null, { status: 204 });
}

/** The hosted page's door: the ticket it redeemed names the device, and the
 *  session joins that device's room as a controller. */
async function connectController(request: Request, env: Env): Promise<Response> {
  const protocols = (request.headers.get("Sec-WebSocket-Protocol") ?? "")
    .split(",")
    .map((entry) => entry.trim());
  const named = protocols.find((entry) => entry.startsWith(AUTH_PROTOCOL_PREFIX));
  const ticket = named?.slice(AUTH_PROTOCOL_PREFIX.length) ?? new URL(request.url).searchParams.get("ticket");
  if (ticket === null || ticket === "") {
    return error(401, "unauthorized", "A pairing ticket is required.");
  }
  const deviceId = await ticketDevice(env, ticket);
  if (deviceId === null) {
    return error(401, "unauthorized", "The pairing ticket is unknown or expired.");
  }
  if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
    return error(426, "upgrade_required", "This endpoint requires a WebSocket upgrade.");
  }
  if (!protocols.includes(REMOTE_PROTOCOL) && new URL(request.url).searchParams.get("ticket") === null) {
    return error(400, "unsupported_protocol", `This endpoint speaks ${REMOTE_PROTOCOL}.`);
  }

  const headers = new Headers(request.headers);
  headers.delete("Authorization");
  headers.set(RELAY_ROLE_HEADER, "controller");
  headers.set(RELAY_CONNECTION_HEADER, crypto.randomUUID());
  const stub = env.RELAY.get(env.RELAY.idFromName(deviceId));
  return stub.fetch(new Request(request.url, { method: "GET", headers }));
}

async function forwardSocket(
  request: Request,
  env: Env,
  deviceId: string,
  role: SocketRole,
): Promise<Response> {
  const authorized = (await authenticateDevice(request, env, deviceId)) !== null;
  if (!authorized) {
    return error(401, "unauthorized", `A valid ${role} credential is required.`);
  }
  if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
    return error(426, "upgrade_required", "This endpoint requires a WebSocket upgrade.");
  }

  const headers = new Headers(request.headers);
  headers.delete("Authorization");
  headers.set(RELAY_ROLE_HEADER, role);
  const stub = env.RELAY.get(env.RELAY.idFromName(deviceId));
  return stub.fetch(new Request(request.url, { method: "GET", headers }));
}

async function authenticateDevice(
  request: Request,
  env: Env,
  deviceId: string,
): Promise<DeviceRecord | null> {
  const credential = bearerToken(request);
  if (credential === null) {
    return null;
  }
  const record = await loadDevice(env, deviceId);
  if (record === null) {
    return null;
  }
  return record.credentialHash === (await sha256Hex(credential)) ? record : null;
}

async function ticketDevice(env: Env, ticket: string): Promise<string | null> {
  return env.PAIRING.get(ticketKey(await sha256Hex(ticket)));
}

async function loadDevice(env: Env, deviceId: string): Promise<DeviceRecord | null> {
  const raw = await env.PAIRING.get(deviceKey(deviceId));
  if (raw === null) {
    return null;
  }
  try {
    return JSON.parse(raw) as DeviceRecord;
  } catch {
    return null;
  }
}

function isCapabilityList(value: unknown): value is string[] {
  return (
    Array.isArray(value) &&
    value.length <= MAX_CAPABILITIES &&
    value.every(
      (entry) => typeof entry === "string" && entry.length > 0 && entry.length <= MAX_CAPABILITY_LENGTH,
    )
  );
}

async function readJsonObject(request: Request): Promise<Record<string, unknown> | null> {
  try {
    const parsed: unknown = await request.json();
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return null;
    }
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}

function bearerToken(request: Request): string | null {
  const header = request.headers.get("Authorization");
  if (header === null) {
    return null;
  }
  return /^Bearer\s+(.+)$/i.exec(header)?.[1] ?? null;
}

function deviceKey(deviceId: string): string {
  return `device:${deviceId}`;
}

function pairKey(code: string): string {
  return `pair:${code}`;
}

function ticketKey(ticketHash: string): string {
  return `ticket:${ticketHash}`;
}

function relayOrigin(request: Request): string {
  const url = new URL(request.url);
  return `${url.protocol === "https:" ? "wss:" : "ws:"}//${url.host}`;
}

function randomPairCode(): string {
  const value = crypto.getRandomValues(new Uint32Array(1))[0] ?? 0;
  return String(value % 1_000_000).padStart(6, "0");
}

function randomToken(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return base64Url(bytes);
}

function mintControlTicket(deviceId: string): string {
  const header = base64Url(
    new TextEncoder().encode(JSON.stringify({ alg: "none", typ: "JWT" })),
  );
  const payload = base64Url(
    new TextEncoder().encode(
      JSON.stringify({
        sub: deviceId,
        exp: Math.floor(Date.now() / 1000) + TICKET_TTL_SECONDS,
      }),
    ),
  );
  return `${header}.${payload}.${randomToken(32)}`;
}

function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

function error(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}
