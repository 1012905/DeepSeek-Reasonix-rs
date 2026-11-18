import { ref, toValue, type MaybeRefOrGetter } from "vue";
import type { AgentPort, RoleAssignments, RoleOverride } from "../port/port";

// The global assignment and the entries that outrank it are two reads of one
// subject; showing the first without the second is how a setting appears to do
// nothing.
export function useRoles(port: MaybeRefOrGetter<AgentPort>) {
  const roles = ref<RoleAssignments | null>(null);
  const overrides = ref<Record<string, RoleOverride[]>>({});
  const loadRoles = () => {
    const agent = toValue(port);
    agent.roles().then((next) => { roles.value = next; }).catch(() => { roles.value = null; });
    agent.roleOverrides().then((next) => { overrides.value = next; }).catch(() => { overrides.value = {}; });
  };
  return { roles, overrides, loadRoles };
}