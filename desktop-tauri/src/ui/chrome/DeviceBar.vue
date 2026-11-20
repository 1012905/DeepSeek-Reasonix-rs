<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import type { HubPort } from "../../port/hub";
import type { DeviceSelf } from "../../port/share";
import { setReachable as publishReachable, setViewer } from "../../state/viewer";
import RMark from "../RMark.vue";

const props = defineProps<{ hub: HubPort }>();

const CHECK_MS = 10_000;

const self = ref<DeviceSelf | null>(null);
const reachable = ref(true);
const arming = ref(false);
let timer: number | null = null;
let arm: number | null = null;
let alive = true;

async function read(): Promise<boolean> {
  try {
    const d = await props.hub.device();
    if (!alive) return false;
    self.value = d;
    setViewer(d);
    reachable.value = true;
    publishReachable(true);
    return d !== null;
  } catch {
    if (alive) {
      reachable.value = false;
      publishReachable(false);
    }
    return true;
  }
}

onMounted(() => {
  void read().then((device) => {
    if (alive && device) timer = window.setInterval(() => void read(), CHECK_MS);
  });
});

onUnmounted(() => {
  alive = false;
  if (timer !== null) window.clearInterval(timer);
  if (arm !== null) window.clearTimeout(arm);
});

function leave() {
  if (arm !== null) window.clearTimeout(arm);
  if (!arming.value) {
    arming.value = true;
    arm = window.setTimeout(() => { arming.value = false; }, 3000);
    return;
  }
  void props.hub.leaveDevice().finally(() => window.location.reload());
}
</script>

<template>
  <div v-if="self" class="devicebar" :data-down="reachable ? undefined : ''" role="status">
    <i aria-hidden="true" />
    <span class="db-what">
      {{ reachable ? t("remote.controlling_machine", { machine: self.machine }) : t("remote.lost_the_connection_to_machine", { machine: self.machine }) }}
    </span>
    <small>{{ t("remote.this_is_device_n", { n: self.ordinal }) }}</small>
    <RMark v-if="!reachable" class="plan-spin db-spin" />
    <em class="db-retry">{{ t("remote.reconnecting") }}</em>
    <button :data-action="arming ? 'device.leave' : 'device.ask-leave'" :data-armed="arming ? '' : undefined" @click="leave()">
      {{ arming ? t("remote.disconnect_2") : t("remote.disconnect") }}
    </button>
  </div>
</template>
