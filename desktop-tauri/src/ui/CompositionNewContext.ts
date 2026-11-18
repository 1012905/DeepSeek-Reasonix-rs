import type { InjectionKey } from "vue";

export const CompositionNewContext: InjectionKey<((id: string) => void) | null> = Symbol("CompositionNewContext");