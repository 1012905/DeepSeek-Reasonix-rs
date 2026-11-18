<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import HeldPeerMessage from "./HeldPeerMessage.vue";
import { peerLabel } from "./peerKeys";
import PeerSettings from "./PeerSettings.vue";
import { usePeers } from "./usePeers";
import type { PeersSettingsStore } from "./usePeersSettings";

const POLL_MS = 2000;

const props = defineProps<{ port: AgentPort; settings: PeersSettingsStore }>();

const { state, failed, refresh } = usePeers(() => props.port, POLL_MS);
const to = ref("");
const text = ref("");
const busy = ref(false);
const error = ref("");
const sent = ref("");

const target = computed(() => to.value || state.value.peers[0]?.key || "");

const onTarget = (e: Event) => { to.value = (e.target as HTMLSelectElement).value; };
const onText = (e: Event) => { text.value = (e.target as HTMLTextAreaElement).value; };

const send = async () => {
  busy.value = true;
  error.value = "";
  sent.value = "";
  try {
    const receipt = await props.port.sendPeerMessage(target.value, text.value);
    sent.value = t("remote.delivered_to_to_it_is", { to: peerLabel(receipt.to) });
    text.value = "";
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const sendNow = () => void send();
const onDone = () => void refresh();
const onSaved = () => void refresh();
</script>

<template>
  <PeerSettings :port="port" :settings="settings" :on-saved="onSaved" />
  <div v-if="failed && !state.enabled" class="empty">{{ t("remote.could_not_read_the_state") }}</div>
  <div v-else-if="state.enabled" class="box">
    <HeldPeerMessage v-for="h in state.held" :key="h.id" :port="port" :held="h" :on-done="onDone" />
    <div v-if="state.peers.length === 0" class="peerhead">{{ t("remote.no_other_session_on_this") }}</div>
    <div v-else class="peerform">
      <label class="peerhead" for="peer-to">{{ t("remote.send_to") }}</label>
      <select id="peer-to" class="peerfield" data-action-change="peers.target" :aria-label="t('remote.send_to')" :value="target" @change="onTarget">
        <option v-for="p in state.peers" :key="p.key" :value="p.key">{{ peerLabel(p) }}</option>
      </select>
      <textarea
        class="peerfield"
        data-action-change="peers.text"
        :aria-label="t('remote.message')"
        :rows="3"
        :value="text"
        :disabled="busy"
        :placeholder="t('remote.write_what_to_tell_the')"
        @input="onText"
      />
      <div v-if="error" class="why">{{ error }}</div>
      <div v-if="sent" class="peerhead">{{ sent }}</div>
      <div class="acts">
        <button class="act" data-action="peers.send" data-primary="true" :disabled="busy || !text.trim() || !target" @click="sendNow">
          {{ t(busy ? "remote.sending" : "common.send") }}
        </button>
      </div>
    </div>
  </div>
</template>
