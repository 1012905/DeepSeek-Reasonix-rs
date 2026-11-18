<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { copyText } from "../CopyButton.vue";
import { useDismiss } from "../dismiss";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { HubPort } from "../../port/hub";
import type { CloudDevice, CloudRemoteStatus, PairedDevice } from "../../port/share";
import { useShare } from "../../state/share";
import Switch from "../Switch.vue";
import TooltipButton from "../TooltipButton.vue";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<{ hub: HubPort; onError?: (e: unknown) => void }>();

const open = ref(false);
const failure = ref("");
const box = ref<HTMLDivElement | null>(null);
useDismiss(open, box, () => { open.value = false; });

const share = useShare(props.hub);
const shareOpen = computed(() => share.st.value?.open ?? false);

// Opening the card is asking for a code, once per opening: after a phone
// spends it, the next one is asked for by hand.
let minted = false;
watch(open, (now) => {
  if (!now) {
    minted = false;
    failure.value = "";
    return;
  }
  void share.refresh()
    .then((st) => {
      if (minted) return;
      minted = true;
      if (st?.cloudRemote?.online && !share.cloudOffer.value) void share.newCloudCode().catch((e: unknown) => { failure.value = reason(e); });
      else if (st?.open) void share.newCode().catch((e: unknown) => { failure.value = reason(e); });
    })
    .catch((e: unknown) => { failure.value = reason(e); });
});

const localNote = usePresenceNote(() => share.st.value?.devices, open);
const cloudNote = useCloudPresenceNote(() => share.st.value?.cloudDevices, open);
const note = computed(() => cloudNote.value || localNote.value);

const online = computed(() => {
  const st = share.st.value;
  if (!st) return 0;
  return st.devices.filter((d) => d.online).length + st.cloudDevices.length;
});

function presenceNote(prev: PairedDevice[], next: PairedDevice[]): string {
  let said = "";
  next.forEach((d, i) => {
    const was = prev.find((p) => p.id === d.id);
    if (d.online && !was?.online) said = t("remote.device_n_connected", { n: i + 1 });
    else if (!d.online && was?.online) said = t("remote.device_n_disconnected", { n: i + 1 });
  });
  prev.forEach((p, i) => {
    if (!next.some((d) => d.id === p.id)) said = t("remote.device_n_disconnected", { n: i + 1 });
  });
  return said;
}

function cloudPresenceNote(prev: CloudDevice[], next: CloudDevice[]): string {
  if (next.find((device) => !prev.some((was) => was.id === device.id))) return t("remote.web_studio_connected");
  if (prev.find((device) => !next.some((now) => now.id === device.id))) return t("remote.web_studio_disconnected");
  return "";
}

function usePresenceNote(read: () => PairedDevice[] | undefined, showing: { value: boolean }) {
  const said = ref("");
  let before: PairedDevice[] | null = null;
  let timer: number | null = null;
  const stop = watch(read, (devices) => {
    if (!devices) return;
    const prev = before;
    before = devices;
    if (!prev || showing.value) return;
    const line = presenceNote(prev, devices);
    if (!line) return;
    said.value = line;
    if (timer !== null) window.clearTimeout(timer);
    timer = window.setTimeout(() => { said.value = ""; }, 3200);
  }, { immediate: true });
  onUnmounted(() => { stop(); if (timer !== null) window.clearTimeout(timer); });
  return computed(() => (showing.value ? "" : said.value));
}

function useCloudPresenceNote(read: () => CloudDevice[] | undefined, showing: { value: boolean }) {
  const said = ref("");
  let before: CloudDevice[] | null = null;
  let timer: number | null = null;
  const stop = watch(read, (devices) => {
    if (!devices) return;
    const prev = before;
    before = devices;
    if (!prev || showing.value) return;
    const line = cloudPresenceNote(prev, devices);
    if (!line) return;
    said.value = line;
    if (timer !== null) window.clearTimeout(timer);
    timer = window.setTimeout(() => { said.value = ""; }, 3200);
  }, { immediate: true });
  onUnmounted(() => { stop(); if (timer !== null) window.clearTimeout(timer); });
  return computed(() => (showing.value ? "" : said.value));
}

function cloudRemoteNote(status: CloudRemoteStatus | undefined): string {
  if (status?.online) return t("remote.internet_access_requires_the_same");
  switch (status?.reason) {
    case "relay_unreachable":
      return t("remote.the_relay_cannot_be_reached");
    case "relay_refused":
      return t("remote.the_relay_refused_this_devices");
    default:
      return t("remote.sign_in_to_reasonix_to");
  }
}

const copied = ref(false);
const arming = ref("");
let arm: number | null = null;
onUnmounted(() => { if (arm !== null) window.clearTimeout(arm); });

const st = computed(() => share.st.value);
const noNetwork = computed(() => (st.value?.addresses.length ?? 0) === 0);
const chosen = computed(() => {
  const s = st.value;
  if (!s) return "";
  return share.ip.value || (s.open ? (s.origin ?? "").replace(/^https?:\/\//, "").replace(/:\d+$/, "") : "") || s.addresses[0]?.ip || "";
});

const phones = computed(() => {
  const s = st.value;
  if (!s) return 0;
  return s.cloudDevices.length + (s.open ? s.devices.filter((d) => d.online).length : 0);
});

function clock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

const deviceLabel = (i: number) => t("remote.device_n", { n: i + 1 });

function copy(url: string) {
  void copyText(url).then(() => {
    copied.value = true;
    window.setTimeout(() => { copied.value = false; }, 1600);
  }).catch(() => {});
}

function disconnect(id: string) {
  if (arm !== null) window.clearTimeout(arm);
  if (arming.value !== id) {
    arming.value = id;
    arm = window.setTimeout(() => { arming.value = ""; }, 3000);
    return;
  }
  arming.value = "";
  void share.revoke(id).catch((e: unknown) => { failure.value = reason(e); });
}

function addressLabel(a: { ip: string; kind: string; interface: string }): string {
  if (a.kind === "tailnet") return `${a.ip} · Tailscale`;
  if (a.kind === "virtual") return `${a.ip} · ${t("remote.virtual_adapter")}`;
  return `${a.ip} · ${a.interface}`;
}

function toggle() {
  void share.toggle().catch((e: unknown) => { failure.value = reason(e); });
}
</script>

<template>
  <div v-if="st" class="phonepop" ref="box">
    <TooltipButton
      class="thbtn phone-action"
      data-action="share.card"
      :data-live="shareOpen ? '' : undefined"
      :data-online="online > 0 ? '' : undefined"
      :aria-expanded="open"
      :aria-label="t('remote.device_access')"
      :title="online > 0 ? t('remote.device_access_n_online', { n: online }) : t('remote.device_access')"
      @click="open = !open"
    >
      <StudioIcon name="qr" />
      <b v-if="online > 0" class="pc-count">{{ online }}</b>
    </TooltipButton>

    <div v-if="open" class="phonecard" role="dialog" :aria-label="t('remote.device_access')">
      <header>
        <span>
          <b>{{ t("remote.use_studio_on_your_phone") }}</b>
          <small>{{ t("remote.scan_with_your_phone_to") }}</small>
        </span>
      </header>

      <p v-if="failure" class="pc-err" role="alert">{{ failure }}</p>

      <section v-if="st.cloudDevices.length > 0 || (st.open && st.devices.length > 0)" class="pc-phones">
        <div class="pc-hd">
          <b>{{ t("remote.n_phones_connected", { n: phones }) }}</b>
        </div>
        <div v-for="device in st.cloudDevices" :key="device.id" class="pc-dev" data-online="">
          <i aria-hidden="true" />
          <span>{{ t("remote.device_n", { n: device.ordinal }) }}</span>
          <small>{{ device.name }} · {{ t("remote.from_anywhere") }}</small>
          <button
            :data-action="arming === device.id ? 'share.revoke' : 'share.ask-revoke'"
            :data-target="device.id"
            :data-armed="arming === device.id ? '' : undefined"
            @click="disconnect(device.id)"
          >
            {{ arming === device.id ? t("remote.disconnect_2") : t("remote.disconnect") }}
          </button>
        </div>
        <template v-if="st.open">
          <div v-for="(d, i) in st.devices" :key="d.id" class="pc-dev" :data-online="d.online ? '' : undefined">
            <i aria-hidden="true" />
            <span :title="d.name">{{ deviceLabel(i) }}</span>
            <small>{{ t("remote.same_wifi") }} · {{ d.online ? t("remote.online") : t("remote.last_seen_time", { time: clock(d.lastSeen) }) }}</small>
            <button
              :data-action="arming === d.id ? 'share.revoke' : 'share.ask-revoke'"
              :data-target="d.id"
              :data-armed="arming === d.id ? '' : undefined"
              @click="disconnect(d.id)"
            >
              {{ arming === d.id ? t("remote.disconnect_2") : t("remote.disconnect") }}
            </button>
          </div>
        </template>
      </section>

      <p class="pc-cap">{{ t("remote.how_the_phone_reaches_this") }}</p>

      <section class="pc-route" :data-on="st.open ? '' : undefined">
        <div class="pc-hd">
          <span>
            <b>{{ t("remote.same_wifi") }}</b>
            <small>{{ chosen ? t("remote.ip_fastest", { ip: chosen }) : t("remote.faster_without_the_relay") }}</small>
          </span>
          <Switch
            data-action="share.toggle"
            :on="st.open"
            :busy="share.busy.value || noNetwork"
            :label="t('remote.allow_local_network_access')"
            :on-click="toggle"
          />
        </div>

        <div v-if="st.open" class="pc-code">
          <template v-if="share.offer.value">
            <img :src="`data:image/svg+xml;charset=utf-8,${encodeURIComponent(share.offer.value.qr)}`" :alt="t('remote.pairing_qr_code')" width="148" height="148" />
            <span class="pc-side">
              <span class="pc-when">{{ t("remote.local_pairing_valid_until_time", { time: clock(share.offer.value.expires) }) }}</span>
              <span class="pc-acts">
                <button data-action="share.copy" @click="copy(share.offer.value.url)">{{ copied ? t("common.copied") : t("remote.copy_link") }}</button>
                <i aria-hidden="true" />
                <button data-action="share.offer" :disabled="share.busy.value" @click="share.newCode()">{{ t("common.new_code") }}</button>
              </span>
            </span>
          </template>
          <button v-else class="pc-mint" data-action="share.offer" :disabled="share.busy.value" @click="share.newCode()">
            {{ t("remote.show_local_network_qr_code") }}
          </button>
        </div>

        <label v-if="st.addresses.length > 1" class="pc-row">
          <span>{{ t("common.network") }}</span>
          <select data-size="sm" data-action="share.address" :value="chosen" :disabled="share.busy.value" @change="share.pick(($event.target as HTMLSelectElement).value)">
            <option v-for="a in st.addresses" :key="a.ip" :value="a.ip">{{ addressLabel(a) }}</option>
          </select>
        </label>
        <p v-if="st.open && st.devices.length === 0" class="pc-none">{{ t("remote.no_phone_connected_yet") }}</p>
      </section>

      <section class="pc-route" :data-on="st.cloudRemote?.online ? '' : undefined">
        <div class="pc-hd">
          <span>
            <b>{{ t("remote.from_anywhere") }}</b>
            <small>{{ t("remote.needs_signing_in") }}</small>
          </span>
        </div>
        <div v-if="st.cloudRemote?.online" class="pc-code" data-cloud="">
          <template v-if="share.cloudOffer.value">
            <img :src="`data:image/svg+xml;charset=utf-8,${encodeURIComponent(share.cloudOffer.value.qr)}`" :alt="t('remote.internet_connection_qr_code')" width="148" height="148" />
            <span class="pc-side">
              <span class="pc-when">{{ t("remote.scan_with_your_phone_works") }}</span>
              <span class="pc-acts">
                <button data-action="share.cloud-copy" @click="copy(share.cloudOffer.value.url)">{{ copied ? t("common.copied") : t("remote.copy_link") }}</button>
                <i aria-hidden="true" />
                <button data-action="share.cloud-offer" :disabled="share.busy.value" @click="share.newCloudCode()">{{ t("remote.refresh_qr_code") }}</button>
              </span>
            </span>
          </template>
          <button v-else class="pc-mint" data-action="share.cloud-offer" :disabled="share.busy.value" @click="share.newCloudCode()">
            {{ share.busy.value ? t("remote.creating_qr_code") : t("remote.show_internet_connection_qr_code") }}
          </button>
        </div>
        <p v-else class="pc-note">{{ cloudRemoteNote(st.cloudRemote) }}</p>
      </section>
    </div>

    <div v-if="note" class="pc-note-pop" role="status">{{ note }}</div>
  </div>
</template>
