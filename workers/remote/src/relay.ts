export type SocketRole = "desktop" | "controller";

export const RELAY_ROLE_HEADER = "x-reasonix-role";
export const RELAY_PROTOCOL_HEADER = "x-reasonix-protocol";
export const RELAY_CONNECTION_HEADER = "x-reasonix-connection";

// The hosted browser speaks this subprotocol; the desktop's Go host speaks the
// gateway envelope below and names no protocol.
export const REMOTE_PROTOCOL = "reasonix.remote.v1";

const MAX_MESSAGE_BYTES = 64 * 1024;
const CLOSE_MESSAGE_TOO_BIG = 1009;
const CLOSE_UNSUPPORTED_DATA = 1003;
const CLOSE_DEVICE_DISCONNECTED = 4004;
const CLOSE_REPLACED = 4401;
const CLOSE_REVOKED = 4403;

const HEARTBEAT_MS = 25_000;

const DESKTOP: SocketRole = "desktop";
const CONTROLLER: SocketRole = "controller";

// The gateway envelope the Go host reads and writes: one JSON object per text
// frame, sealed payloads inside it. The room never inspects those payloads.
interface GatewayMessage {
  type?: string;
  connectionId?: string;
  scopes?: string[];
  payload?: string;
  to?: string;
}

const SCOPES = ["desktop"];

/**
 * One room per device. It holds at most one desktop socket and any number of
 * controller sockets; payloads are forwarded opaquely in the gateway envelope
 * the Go host and the hosted page already speak.
 */
export class RelayRoom {
  constructor(private readonly ctx: DurableObjectState) {}

  async fetch(request: Request): Promise<Response> {
    const { pathname } = new URL(request.url);
    if (request.method === "POST" && pathname === "/revoke") {
      this.closeRole(DESKTOP, CLOSE_REVOKED, "device revoked");
      this.closeRole(CONTROLLER, CLOSE_REVOKED, "device revoked");
      return new Response(null, { status: 204 });
    }

    const header = request.headers.get(RELAY_ROLE_HEADER);
    if (header !== DESKTOP && header !== CONTROLLER) {
      return new Response("unknown relay role", { status: 400 });
    }
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected websocket upgrade", { status: 426 });
    }

    const role: SocketRole = header === DESKTOP ? DESKTOP : CONTROLLER;
    if (role === DESKTOP) {
      this.closeRole(DESKTOP, CLOSE_REPLACED, "superseded by a newer connection");
    }

    const connectionId = request.headers.get(RELAY_CONNECTION_HEADER) ?? crypto.randomUUID();
    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    this.ctx.acceptWebSocket(server, [role, connectionId]);
    if (role === DESKTOP) {
      this.greet(server);
    } else {
      this.toDesktop({ type: "controller_connected", connectionId, scopes: SCOPES });
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const size =
      typeof message === "string" ? new TextEncoder().encode(message).byteLength : message.byteLength;
    if (size > MAX_MESSAGE_BYTES) {
      socket.close(CLOSE_MESSAGE_TOO_BIG, "message exceeds 64 KiB");
      return;
    }
    if (typeof message !== "string") {
      socket.close(CLOSE_UNSUPPORTED_DATA, "binary frames are not part of this protocol");
      return;
    }

    const role = this.roleOf(socket);
    if (role === null) {
      socket.close(1011, "unrecognized socket");
      return;
    }

    if (role === DESKTOP) {
      this.fromDesktop(message);
      return;
    }
    const connectionId = this.connectionOf(socket);
    if (connectionId === null) {
      socket.close(1011, "controller without a connection id");
      return;
    }
    this.toDesktop({ type: "controller_message", connectionId, scopes: SCOPES, payload: message });
  }

  async webSocketClose(
    socket: WebSocket,
    _code: number,
    _reason: string,
    _wasClean: boolean,
  ): Promise<void> {
    this.departed(socket);
  }

  async webSocketError(socket: WebSocket): Promise<void> {
    this.departed(socket);
  }

  private departed(socket: WebSocket): void {
    const role = this.roleOf(socket);
    if (role === DESKTOP) {
      if (!this.hasOtherDesktop(socket)) {
        this.closeRole(CONTROLLER, CLOSE_DEVICE_DISCONNECTED, "device disconnected");
      }
      return;
    }
    const connectionId = this.connectionOf(socket);
    if (connectionId !== null && this.hasOtherDesktop(socket)) {
      this.toDesktop({ type: "controller_disconnected", connectionId });
    }
  }

  /** The hello the Go host negotiates from: it lists live controllers and names
   *  the heartbeat token the host echoes back on its own tick. */
  private greet(socket: WebSocket): void {
    socket.send(
      JSON.stringify({
        type: "relay_hello",
        controllers: this.ctx
          .getWebSockets(CONTROLLER)
          .map((peer) => this.connectionOf(peer))
          .filter((id): id is string => id !== null),
        heartbeat: crypto.randomUUID(),
        heartbeatMs: HEARTBEAT_MS,
      }),
    );
  }

  private fromDesktop(message: string): void {
    let frame: GatewayMessage;
    try {
      frame = JSON.parse(message) as GatewayMessage;
    } catch {
      return; // heartbeat echoes and pings are not JSON, and are not for us
    }
    if (frame.type === "disconnect_controller" && typeof frame.connectionId === "string") {
      this.closeController(frame.connectionId, CLOSE_DEVICE_DISCONNECTED, "device closed the session");
      return;
    }
    if (typeof frame.to !== "string" || typeof frame.payload !== "string") {
      return;
    }
    const target = this.ctx
      .getWebSockets(CONTROLLER)
      .find((peer) => this.connectionOf(peer) === frame.to);
    if (target === undefined) {
      return;
    }
    try {
      target.send(frame.payload);
    } catch {
      // The controller may have closed since the socket list was read.
    }
  }

  private toDesktop(frame: Record<string, unknown>): void {
    for (const peer of this.ctx.getWebSockets(DESKTOP)) {
      try {
        peer.send(JSON.stringify(frame));
      } catch {
        // A replaced desktop is already gone.
      }
    }
  }

  private closeController(connectionId: string, code: number, reason: string): void {
    for (const peer of this.ctx.getWebSockets(CONTROLLER)) {
      if (this.connectionOf(peer) === connectionId) {
        try {
          peer.close(code, reason);
        } catch {
          // Already closing.
        }
      }
    }
  }

  /** A replaced desktop must not take the controllers down with it. */
  private hasOtherDesktop(socket: WebSocket): boolean {
    return this.ctx.getWebSockets(DESKTOP).some((peer) => peer !== socket);
  }

  private roleOf(socket: WebSocket): SocketRole | null {
    const tag = this.ctx.getTags(socket)[0];
    if (tag === DESKTOP) return DESKTOP;
    if (tag === CONTROLLER) return CONTROLLER;
    return null;
  }

  private connectionOf(socket: WebSocket): string | null {
    return this.ctx.getTags(socket)[1] ?? null;
  }

  private closeRole(role: SocketRole, code: number, reason: string): void {
    for (const socket of this.ctx.getWebSockets(role)) {
      try {
        socket.close(code, reason);
      } catch {
        // A socket that is already closing rejects close().
      }
    }
  }
}
