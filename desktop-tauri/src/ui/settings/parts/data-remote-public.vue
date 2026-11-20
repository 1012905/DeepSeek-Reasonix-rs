<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { HubPort } from "../../../port/hub";
import type { ShareAddress, ShareStatus } from "../../../port/share";
import Group from "../../Group.vue";
import Switch from "../../Switch.vue";

// The direct door on the internet: the same share on a globally routable IPv6,
// behind a self-signed certificate. No relay and no domain are involved; the
// browser warns about the certificate once, and the router has to let the port
// in. When it cannot be reached the relay card next door is the fallback.
const props = defineProps<{ hub: HubPort; onError: (e: unknown) => void }>();

const st = ref<ShareStatus | null>(null);
const ip = ref("");
const busy = ref(false);
const error = ref("");

async function read(): Promise<void> {
  try {
    st.value = await props.hub.shareStatus();
    error.value = "";
  } catch (e) {
    st.value = null;
    error.value = reason(e);
  }
}

watch(() => props.hub, () => { void read(); }, { immediate: true });

const publicList = computed<ShareAddress[]>(() => st.value?.publicAddresses ?? []);
const opened = computed(() => st.value?.public === true && st.value?.open === true);
const chosen = computed(() => ip.value || publicList.value[0]?.ip || "");
const noPublic = computed(() => publicList.value.length === 0);

async function run(step: () => Promise<void>): Promise<void> {
  busy.value = true;
  try {
    await step();
  } catch (e) {
    props.onError(e);
  } finally {
    busy.value = false;
  }
}

const toggle = () => run(async () => {
  if (opened.value) {
    st.value = await props.hub.closeShare();
    return;
  }
  if (chosen.value === "") return;
  st.value = await props.hub.openPublicShare(chosen.value);
});

const pick = (next: string) => {
  ip.value = next;
  if (!opened.value) return;
  void run(async () => { st.value = await props.hub.openPublicShare(next); });
};

const fingerprint = computed(() => (st.value?.certFingerprint ?? "").slice(0, 32).toUpperCase().replace(/(.{8})/g, "$1 ").trim());
</script>

<template>
  <Group id="public-door" :title="t('remote.public_door')" :hint="t('remote.public_door_hint')">
    <p v-if="error" class="why" role="alert">{{ error }}</p>
    <p v-if="noPublic" class="rmthint">{{ t("remote.no_public_address") }}</p>

    <template v-else>
      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("remote.open_direct_door") }}</span>
          <span class="ds">{{ t("remote.direct_door_reaches_anywhere") }}</span>
        </span>
        <Switch data-action="share.public-toggle" :on="opened" :busy="busy" :label="t('remote.open_direct_door')" :on-click="toggle" />
      </div>

      <label v-if="publicList.length > 1 && !opened" class="rmtf">
        <span>{{ t("common.network") }}</span>
        <select data-action="share.public-address" :value="chosen" :disabled="busy" @change="pick(($event.target as HTMLSelectElement).value)">
          <option v-for="a in publicList" :key="a.ip" :value="a.ip">{{ a.ip }} · {{ a.interface }}</option>
        </select>
      </label>

      <template v-if="opened">
        <p class="rmthint" dir="ltr">{{ st?.origin }}</p>
        <p class="rmthint">{{ t("remote.browser_warns_about_certificate") }}</p>
        <p class="rmthint" data-action="share.public-fingerprint">{{ t("remote.certificate_fingerprint", { fp: fingerprint }) }}</p>
        <p class="rmthint">{{ t("remote.router_must_allow_the_port") }}</p>
      </template>
      <p class="rmthint">{{ t("remote.relay_fallback_hint") }}</p>
    </template>
  </Group>
</template>
