import { RELAY_ROLE_HEADER, RelayRoom, type SocketRole } from "./relay";

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

const DEVICE_PAIR_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/pair$/;
const DEVICE_CONNECT_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/connect$/;
const DEVICE_CONTROL_ROUTE = /^\/v1\/devices\/([A-Za-z0-9_-]+)\/control$/;
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

    const control = DEVICE_CONTROL_ROUTE.exec(pathname);
    if (request.method === "GET" && control !== null) {
      return forwardSocket(request, env, control[1]!, "controller");
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

async function forwardSocket(
  request: Request,
  env: Env,
  deviceId: string,
  role: SocketRole,
): Promise<Response> {
  const authorized =
    role === "desktop"
      ? (await authenticateDevice(request, env, deviceId)) !== null
      : await authenticateTicket(request, env, deviceId);
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

async function authenticateTicket(request: Request, env: Env, deviceId: string): Promise<boolean> {
  const ticket = bearerToken(request);
  if (ticket === null) {
    return false;
  }
  const ticketDevice = await env.PAIRING.get(ticketKey(await sha256Hex(ticket)));
  return ticketDevice === deviceId;
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
