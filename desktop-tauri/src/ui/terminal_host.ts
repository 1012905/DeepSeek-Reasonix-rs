import type { InjectionKey } from "vue";

// The machine a remote project lives on, or "" for this one. A terminal opened
// from a remote pane runs there, and its header says so.
export const TerminalHostContext: InjectionKey<string> = Symbol("TerminalHostContext");