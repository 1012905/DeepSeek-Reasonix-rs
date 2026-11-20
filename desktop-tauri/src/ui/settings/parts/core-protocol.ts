import { computed, ref } from "vue";

// Which protocol each account is showing, as picked on the services page.
//
// One map for the whole window: the model list reads it to decide whose models
// an account offers, and the services list reads it to decide which door it is
// editing. Two copies would let the two pages disagree about one account, which
// is exactly the state this control exists to make impossible.

const chosen = ref<Record<string, string>>({});

export const protocols = computed(() => chosen.value);

export function rememberProtocol(key: string, kind: string): void {
  chosen.value = { ...chosen.value, [key]: kind };
}

// A vanished account simply stops matching; nothing has to be cleaned up.
export function forgetProtocol(key: string): void {
  if (!(key in chosen.value)) return;
  const next = { ...chosen.value };
  delete next[key];
  chosen.value = next;
}
