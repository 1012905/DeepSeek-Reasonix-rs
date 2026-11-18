<script lang="ts">
import type { ProviderEntry } from "../port/port";

export function hasEndpointSwitches(entry: ProviderEntry): boolean {
  return !!(entry.canWebSearch || entry.canSetThinking || entry.canSetContinuation);
}
</script>

<script setup lang="ts">
import type { VNodeChild } from "vue";
import { t } from "../i18n";

// How a turn's context reaches the next one. Auto is vendor detection, which is
// the only honest answer for an endpoint nobody has characterised; the other two
// are what a reader picks when the endpoint has already contradicted it.
const CONTINUATIONS: ReadonlyArray<readonly [string, string]> = [
  ["", "common.auto"],
  ["stateful", "models.by_reference"],
  ["stateless", "models.replay_each_turn"],
];

const CONTINUATION_WHY: Record<string, string> = {
  "": "models.decided_by_the_endpoints_vendor",
  stateful: "models.sends_only_the_new_turn",
  stateless: "models.replays_the_whole_history_every",
};

// The declarations that live on the stored wire, held in the same draft as the
// rest of the form: nothing here is written until Save.
defineProps<{
  entry: ProviderEntry;
  label: string;
  shape: VNodeChild;
  search: boolean;
  sendsThinking: boolean;
  continuation: string;
  onSearch: (on: boolean) => void;
  onThinking: (send: boolean) => void;
  onContinuation: (mode: string) => void;
  disabled: boolean;
}>();
</script>

<template>
  <div class="pdetail-options">
    <div v-if="entry.canWebSearch" class="vway">
      <span class="lb">{{ t("models.web_search") }}</span>
      <div class="seg" role="group" :aria-label="t('models.web_search_for_name', { name: label })">
        <button v-for="on in [true, false]" :key="String(on)" type="button" data-action="provider.draft" :data-value="`web-search:${on}`" :aria-pressed="search === on" :disabled="disabled" @click="onSearch(on)">{{ t(on ? "models.on" : "common.off") }}</button>
      </div>
      <span class="why">{{ t("models.the_endpoint_runs_the_search") }}</span>
    </div>
    <div v-if="entry.canSetThinking" class="vway">
      <span class="lb">{{ t("models.thinking_parameter") }}</span>
      <div class="seg" role="group" :aria-label="t('models.thinking_parameter_for_name', { name: label })">
        <button v-for="on in [true, false]" :key="String(on)" type="button" data-action="provider.draft" :data-value="`thinking:${on}`" :aria-pressed="sendsThinking === on" :disabled="disabled" @click="onThinking(on)">{{ t(on ? "common.auto" : "models.do_not_send") }}</button>
      </div>
      <span class="why">{{ t(sendsThinking ? "models.some_relays_do_not_accept" : "models.sends_only_the_ordinary_chat") }}</span>
      <component :is="shape" v-if="sendsThinking && shape" />
    </div>
    <div v-if="entry.canSetContinuation" class="vway">
      <span class="lb">{{ t("models.carrying_context") }}</span>
      <div class="seg" role="group" :aria-label="t('models.how_name_carries_context_between', { name: label })">
        <button v-for="[mode, text] in CONTINUATIONS" :key="mode" type="button" data-action="provider.draft" :data-value="`continuation:${mode || 'auto'}`" :aria-pressed="continuation === mode" :disabled="disabled" @click="onContinuation(mode)">{{ t(text) }}</button>
      </div>
      <span class="why">{{ t(CONTINUATION_WHY[continuation] ?? CONTINUATION_WHY[""]) }}</span>
    </div>
  </div>
</template>