import type { InjectionKey } from "vue";
import type { AgentPort } from "../../port/port";

export type SurfaceMediaPort = Pick<AgentPort, "workspaceImageURL">;
export const SurfaceMediaContext: InjectionKey<Partial<SurfaceMediaPort> | undefined> = Symbol("SurfaceMediaContext");