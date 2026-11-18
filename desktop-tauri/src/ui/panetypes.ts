import type { VNodeChild } from "vue";
import type { AgentPort, ContextBreakdown, McpEntry, SessionStatus, WorkspaceInfo } from "../port/port";
import type { HubPort, RuntimeView } from "../port/hub";
import type { ConnectOffer } from "./ConnectNote.vue";
import type { PaneView } from "../state/paneview";

export interface PaneReport {
  status: SessionStatus | null;
  workspace: WorkspaceInfo | null;
  title: string;
  steer: number;
  run: string;
  live: boolean;
  cost: string;
  contextPercent: number | null;
  context: ContextBreakdown | null;
  mcp: McpEntry[];
  wallet: string;
  interactions?: import("../state/session_types").SessionState["interactions"];
  view: PaneView;
  rows: number;
  surfaces: number;
}

export interface PaneProps {
  port: AgentPort;
  hub?: Pick<HubPort, "openWith" | "openWithTargets">;
  rt: RuntimeView;
  title: string;
  active: boolean;
  sideHost: HTMLElement | null;
  side: boolean;
  onFocus: () => void;
  onReport: (id: string, report: PaneReport) => void;
  visible: boolean;
  onSessionChanged: () => void;
  onTurnDone?: (id: string) => void;
  pulse: number;
  findPulse: number;
  findTogglePulse?: number;
  filesPulse?: number;
  terminalPulse?: number;
  viewRequest?: { to: PaneView; n: number };
  sessionRef?: { n: number; id: string; title: string };
  onSettings: (section?: string) => void;
  onFeedback?: () => void;
  onOpenSession?: (path: string) => void;
  needsProject: boolean;
  onOpenProject: () => void;
  onKeepHere: () => void;
  connect?: ConnectOffer;
  theme: string;
  dockW: number;
  dockMax: number;
  onDockW: (w: number) => void;
  manualBrowser?: boolean;
  onManualBrowser?: (on: boolean) => void;
  alert?: VNodeChild;
  failure?: VNodeChild;
}
