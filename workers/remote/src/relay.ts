export type SocketRole = "desktop" | "controller";

export const RELAY_ROLE_HEADER = "x-reasonix-role";

const MAX_MESSAGE_BYTES = 64 * 1024;
const CLOSE_MESSAGE_TOO_BIG = 1009;
const CLOSE_DEVICE_DISCONNECTED = 4004;
const CLOSE_REPLACED = 4401;
const CLOSE_REVOKED = 4403;

const DESKTOP: SocketRole = "desktop";
const CONTROLLER: SocketRole = "controller";

/**
 * One room per device. It holds at most one desktop socket and forwards
 * opaque envelopes to the opposite role; nothing is stored or inspected.
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

    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    this.ctx.acceptWebSocket(server, [role]);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const size =
      typeof message === "string" ? new TextEncoder().encode(message).byteLength : message.byteLength;
    if (size > MAX_MESSAGE_BYTES) {
      socket.close(CLOSE_MESSAGE_TOO_BIG, "message exceeds 64 KiB");
      return;
    }

    const role = this.roleOf(socket);
    if (role === null) {
      socket.close(1011, "unrecognized socket");
      return;
    }

    const target: SocketRole = role === DESKTOP ? CONTROLLER : DESKTOP;
    for (const peer of this.ctx.getWebSockets(target)) {
      try {
        peer.send(message);
      } catch {
        // A peer may have closed since the socket list was read.
      }
    }
  }

  async webSocketClose(
    socket: WebSocket,
    _code: number,
    _reason: string,
    _wasClean: boolean,
  ): Promise<void> {
    if (this.roleOf(socket) === DESKTOP && !this.hasOtherDesktop(socket)) {
      this.closeRole(CONTROLLER, CLOSE_DEVICE_DISCONNECTED, "device disconnected");
    }
  }

  async webSocketError(socket: WebSocket, _error: unknown): Promise<void> {
    if (this.roleOf(socket) === DESKTOP && !this.hasOtherDesktop(socket)) {
      this.closeRole(CONTROLLER, CLOSE_DEVICE_DISCONNECTED, "device disconnected");
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
