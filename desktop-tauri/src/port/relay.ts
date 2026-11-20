// What the desktop says about its own relay registration. Mirrors the routes
// the Go host serves under /remote/relay.

export interface RelayStatus {
  enabled: boolean;
  // This desktop's socket to the relay is live, so a paired page can join now.
  online: boolean;
  deviceId: string;
}

export interface RelayCode {
  // Six digits, single use, and short-lived: the browser trades it for a ticket.
  code: string;
  expiresIn: number;
}

export interface RelayPort {
  relayStatus(): Promise<RelayStatus>;
  // Registers this desktop with the relay and starts holding a socket open.
  relayEnable(): Promise<{ deviceId: string }>;
  relayDisable(): Promise<void>;
  relayPair(): Promise<RelayCode>;
}
