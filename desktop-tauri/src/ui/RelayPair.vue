<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { HubPort } from "../port/hub";
import { onRelayEnded, redeemAndConnect, relayBase, RelayLinkError } from "../port/relay_cloud";
import App from "./App.vue";

const SAID: Record<RelayLinkError["kind"], string> = {
  invalid_code: "relay.err_invalid_code",
  offline: "relay.err_offline",
  protocol: "relay.err_protocol",
  ended: "relay.err_ended",
};

const code = ref("");
const busy = ref(false);
const hub = ref<HubPort | null>(null);
const failure = ref<RelayLinkError["kind"] | "">("");
const other = ref("");
let stop: (() => void) | null = null;

// redeemAndConnect routes same-origin fetch through the link it opens, so an
// attempt after that link ended would redeem through the session that is gone.
// Each attempt therefore starts from this page's own fetch.
const networkFetch = globalThis.fetch;

function say(error: unknown): void {
  if (error instanceof RelayLinkError) {
    failure.value = error.kind;
    other.value = "";
    return;
  }
  failure.value = "";
  other.value = reason(error);
}

function type(event: Event): void {
  const input = event.target as HTMLInputElement;
  const next = input.value.replace(/\D/g, "").slice(0, 6);
  code.value = next;
  // A key that is not a digit is dropped from the model and from the box alike,
  // or the field would show a code the page will not send.
  input.value = next;
}

async function submit(): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  failure.value = "";
  other.value = "";
  try {
    globalThis.fetch = networkFetch;
    const opened = await redeemAndConnect(relayBase(), code.value);
    stop?.();
    stop = onRelayEnded((end) => close(end.kind));
    hub.value = opened;
  } catch (error) {
    say(error);
  } finally {
    busy.value = false;
  }
}

function close(kind: RelayLinkError["kind"]): void {
  hub.value = null;
  code.value = "";
  failure.value = kind;
  other.value = "";
}

onBeforeUnmount(() => { stop?.(); });
</script>

<template>
  <App v-if="hub" :hub="hub" />
  <p v-if="hub" class="sr-only" role="status" aria-live="polite">{{ t("relay.connected") }}</p>

  <div v-else class="askveil">
    <form class="askcard" aria-labelledby="relay-pair-t" @submit.prevent="submit">
      <h2 id="relay-pair-t">{{ t("relay.pair_title") }}</h2>
      <p class="h">{{ t("relay.pair_hint") }}</p>
      <input
        id="relay-code"
        data-action="relay.connect"
        inputmode="numeric"
        pattern="[0-9]*"
        maxlength="6"
        autocomplete="one-time-code"
        autofocus
        :value="code"
        :aria-label="t('relay.code_label')"
        :placeholder="t('relay.code_placeholder')"
        @input="type"
      />
      <p v-if="busy" class="h" role="status" aria-live="polite">{{ t("relay.connecting") }}</p>
      <div v-if="failure || other" class="errbar" role="alert">
        <span>{{ failure ? t(SAID[failure]) : other }}</span>
      </div>
      <div class="askact">
        <button type="submit" data-go :disabled="busy">{{ t("relay.connect") }}</button>
      </div>
    </form>
  </div>
</template>
