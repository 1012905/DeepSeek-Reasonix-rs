<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort, PeerHeld, PeerProject, PeerRemember } from "../port/port";
import { heldLabel } from "./peerKeys";

const props = defineProps<{ port: AgentPort; held: PeerHeld; project?: PeerProject; onDone: () => void; compact?: boolean }>();

const busy = ref(false);
const open = ref(!props.compact);
const error = ref("");
const decide = async (approve: boolean, remember: PeerRemember = "none") => {
  busy.value = true;
  error.value = "";
  try {
    await props.port.resolvePeerMessage(props.held.id, approve, remember);
    props.onDone();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div class="peerheld" role="group" :aria-label="t('remote.message_from_from', { from: heldLabel(held) })">
    <div class="peerrow">
      <span class="peerhead" :title="t('remote.text_from_another_session_from', { from: heldLabel(held) })">
        {{ compact ? t("remote.from_sent_a_message_and", { from: heldLabel(held) }) : t("remote.text_from_another_session_from", { from: heldLabel(held) }) }}
      </span>
      <span class="acts">
        <button v-if="compact" class="act" data-action="peers.expand" :aria-expanded="open" @click="open = !open">{{ t(open ? "common.collapse" : "common.expand") }}</button>
        <button class="act" data-action="peers.deny" :disabled="busy" @click="void decide(false)">{{ t("common.refuse") }}</button>
        <button v-if="project" class="act" data-action="peers.approve-project" :disabled="busy" :aria-describedby="`peer-remember-${held.id}`" @click="void decide(true, 'project')">{{ t("remote.approve_and_remember_project_name", { name: project.name }) }}</button>
        <button class="act" data-action="peers.approve" data-primary="true" :disabled="busy" @click="void decide(true)">{{ t("remote.approve_and_give_it_to") }}</button>
      </span>
    </div>
    <div v-if="project" class="apv-note" :id="`peer-remember-${held.id}`">{{ t("remote.remember_project_remembers_the_project", { name: project.name, path: project.path }) }}</div>
    <pre class="peerbody" :data-clamped="open ? undefined : ''">{{ held.body }}</pre>
    <div v-if="error" class="why">{{ error }}</div>
  </div>
</template>