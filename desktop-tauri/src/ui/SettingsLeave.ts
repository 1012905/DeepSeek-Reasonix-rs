import { inject, toValue, watchEffect, type InjectionKey, type MaybeRefOrGetter } from "vue";

type Guard = (leave: () => void) => void;
type Register = (guard: Guard) => () => void;

/** The sheet's leave guard: a section with an unsaved draft registers itself,
 *  and every navigation asks it first. Null when no section is holding one. */
export const SettingsLeaveContext: InjectionKey<Register | null> = Symbol("SettingsLeaveContext");

export function useSettingsLeave() {
  let guard: Guard | null = null;
  const register: Register = (next) => {
    guard = next;
    return () => { if (guard === next) guard = null; };
  };
  const leave = (next: () => void) => {
    if (guard) guard(next);
    else next();
  };
  return { register, leave };
}

export function useSettingsLeaveGuard(guard: MaybeRefOrGetter<Guard>) {
  const register = inject(SettingsLeaveContext, null);
  watchEffect((onCleanup) => {
    const off = register?.(toValue(guard));
    if (off) onCleanup(off);
  });
}
