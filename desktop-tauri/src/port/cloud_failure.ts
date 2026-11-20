// Why a relay connection could not be made or kept. A browser is told nothing
// about a refused WebSocket upgrade, so these come from what can be observed
// around it, never from the text of an error.
export type RelayFailure =
  | "offline"
  | "unreachable"
  | "origin_rejected"
  | "rate_limited"
  | "refused"
  | "idle"
  | "unknown";

export interface FailureDetail {
  code: RelayFailure;
  retryAfterS?: number;
}

// What the page can offer next. "ended" is not a retry: a ticket the relay
// refused and an origin it rejected are not fixed by asking again.
export type FailureAction = "reconnect" | "ended";

export interface FailureView {
  title: string;
  body: string;
  action: FailureAction;
  waitS: number;
  retryWhenOnline: boolean;
}

export function failureView({ code, retryAfterS }: FailureDetail): FailureView {
  const reconnect = { action: "reconnect" as const, waitS: 0, retryWhenOnline: false };
  switch (code) {
    case "offline":
      return { ...reconnect, retryWhenOnline: true, title: "You're offline", body: "This device has no network connection right now." };
    case "unreachable":
      return {
        ...reconnect,
        title: "Can't reach the remote service",
        body: "This network can't reach the relay. Check the proxy or firewall and try again.",
      };
    case "origin_rejected":
      return {
        ...reconnect,
        action: "ended",
        title: "This address can't use the relay",
        body: "The relay rejected this page's origin, so it cannot reach the computer from here.",
      };
    case "rate_limited": {
      const waitS = retryAfterS && retryAfterS > 0 ? Math.min(Math.ceil(retryAfterS), 600) : 0;
      return {
        action: "reconnect",
        retryWhenOnline: false,
        waitS,
        title: "Too many attempts",
        body: waitS > 0
          ? `Too many requests. Wait ${waitS} s, then connect again.`
          : "Too many requests. Wait a moment, then connect again.",
      };
    }
    case "refused":
      return {
        ...reconnect,
        action: "ended",
        title: "The relay refused this connection",
        body: "The connection ticket may have expired. Pair this computer again to get a new code.",
      };
    case "idle":
      return {
        ...reconnect,
        title: "Inactive for a while",
        body: "The relay released this idle connection. Reconnect to continue.",
      };
    case "unknown":
      return {
        ...reconnect,
        title: "Remote connection failed",
        body: "The remote relay is temporarily unavailable. Try again shortly.",
      };
  }
}

export function retryAfterSeconds(value: string | null): number | undefined {
  if (value === null) return undefined;
  const seconds = Number(value.trim());
  return Number.isFinite(seconds) && seconds > 0 ? seconds : undefined;
}
