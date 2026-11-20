<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { RelayCode, RelayStatus } from "../../../port/relay";
import type { HubPort } from "../../../port/hub";
import type { ShareOffer, ShareStatus } from "../../../port/share";
import { copyText } from "../../CopyButton.vue";
import Group from "../../Group.vue";
import Switch from "../../Switch.vue";

// While the door is open the list is what changes under the reader: a phone
// pairs, the code on screen is spent. Nothing pushes that, so it is read.
const POLL_MS = 3000;

const clock = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
};

// A device is named by when it paired. Its user agent is kept, as the title, but
// read as a name it is a hundred characters and every one starts the same.
const deviceLabel = (i: number) => t("remote.device_n", { n: i + 1 });

const props = defineProps<{ hub: HubPort; port: AgentPort; onError: (e: unknown) => void }>();

const st = ref<ShareStatus | null>(null);
const ip = ref("");
const offer = ref<ShareOffer | null>(null);
const busy = ref(false);
const confirm = ref("");
const remember = ref<boolean | null>(null);
const copied = ref<"" | "done" | "failed">("");
const portDraft = ref<string | null>(null);
const error = ref("");

const relay = ref<RelayStatus | null>(null);
const pairing = ref<RelayCode | null>(null);
const relayBusy = ref(false);

async function readRelay(): Promise<void> {
  try {
    relay.value = await props.port.relayStatus();
  } catch (e) {
    relay.value = null;
    props.onError(e);
  }
}

watch(() => props.port, () => { void readRelay(); }, { immediate: true });

async function read(): Promise<void> {
  try {
    st.value = await props.hub.shareStatus();
    error.value = "";
  } catch (e) {
    st.value = null;
    error.value = reason(e);
  }
}

let timer: ReturnType<typeof setInterval> | undefined;
watch(() => props.hub, (hub) => {
  void read();
  hub.shareRemember().then((r) => { remember.value = r?.remember ?? false; }).catch(() => { remember.value = null; });
  timer = setInterval(() => void read(), POLL_MS);
  return () => { clearInterval(timer); };
}, { immediate: true });

// The kernel withdraws a code once it pairs or lapses; the picture of it has to
// go too, or the next phone scans a code that can no longer work.
watch([st, offer], () => { if (offer.value && st.value && !st.value.offerExpires) offer.value = null; });

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

async function relayRun(step: () => Promise<void>): Promise<void> {
  relayBusy.value = true;
  try {
    await step();
  } catch (e) {
    props.onError(e);
  } finally {
    relayBusy.value = false;
  }
}

// The code and the status that knows it is on offer land together; set one at a
// time, the effect above reads a fresh code against a status that has not heard
// of it yet and drops it.
async function mint(): Promise<void> {
  const next = await props.hub.offerShare();
  st.value = await props.hub.shareStatus();
  offer.value = next;
}

const toggle = () => run(async () => {
  const now = st.value;
  if (now?.open) {
    offer.value = null;
    st.value = await props.hub.closeShare();
    return;
  }
  const at = ip.value || now?.addresses[0]?.ip || "";
  await props.hub.openShare(at);
  await mint();
});

const newCode = () => run(mint);

// Choosing another network while the door is open moves it there: the old
// address stops answering, so the code and every pairing go with it.
const pick = (next: string) => {
  ip.value = next;
  if (!st.value?.open) return;
  void run(async () => {
    offer.value = null;
    await props.hub.openShare(next);
    await mint();
  });
};

// A fixed port is read at the next open, so an open door is moved to it the way
// a network change moves it: the old origin stops answering.
const savePort = (port: number) => run(async () => {
  const next = await props.hub.setSharePort(port);
  st.value = next;
  if (!next.open) return;
  const at = ip.value || next.origin?.replace(/^https?:\/\//, "").replace(/:\d+$/, "") || next.addresses[0]?.ip || "";
  offer.value = null;
  await props.hub.openShare(at);
  await mint();
});

const saveRemember = () => run(async () => {
  const got = await props.hub.setShareRemember(!(remember.value ?? false));
  remember.value = got.remember;
});

const revoke = (id: string) => run(async () => { st.value = await props.hub.revokeDevice(id); });

// The link the code draws, for a device that cannot scan: the same one-time
// credential, so it is copied on request and never shown as text.
const copyLink = (url: string) => {
  copyText(url)
    .then(() => { copied.value = "done"; })
    .catch(() => { copied.value = "failed"; })
    .finally(() => { window.setTimeout(() => { copied.value = ""; }, 1600); });
};

// The line a paired device reads: online is one sentence, last-seen another.
const seenSaid = (d: { online: boolean; pairedAt: string; lastSeen: string }): string =>
  t(d.online ? "remote.paired_time_online" : "remote.paired_time_last_seen_seen", { time: clock(d.pairedAt), seen: clock(d.lastSeen) });

const chosen = computed(() => ip.value || (st.value?.open ? st.value?.origin?.replace(/^https?:\/\//, "").replace(/:\d+$/, "") : "") || st.value?.addresses[0]?.ip || "");
const noNetwork = computed(() => st.value?.addresses.length === 0);

function commitPort(): void {
  const held = portDraft.value;
  if (held === null) return;
  portDraft.value = null;
  const next = held === "" ? 0 : Number(held);
  if (next !== (st.value?.port ?? 0)) void savePort(next);
}

const origin = computed(() => st.value?.origin ?? "");

const relayOn = computed(() => relay.value?.enabled === true);
// A device id is a token, not a name: long enough to tell two apart, short
// enough to sit in a row.
const shortDevice = computed(() => {
  const id = relay.value?.deviceId ?? "";
  return id.length > 12 ? `${id.slice(0, 12)}…` : id;
});
const relayMinutes = computed(() => Math.max(1, Math.round((pairing.value?.expiresIn ?? 0) / 60)));

const enableRelay = () => relayRun(async () => {
  await props.port.relayEnable();
  await readRelay();
});

const disableRelay = () => relayRun(async () => {
  await props.port.relayDisable();
  pairing.value = null;
  await readRelay();
});

const showCode = () => relayRun(async () => {
  pairing.value = await props.port.relayPair();
});
</script>

<template>
  <Group v-if="st" id="phone" :title="t('remote.phone_access')" :hint="t('settings.phone_hint')">
    <div class="share">
      <p v-if="error" class="why" role="alert">{{ error }}</p>
      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("remote.allow_phones_to_connect") }}</span>
          <span class="ds">{{ t(noNetwork ? "remote.this_computer_has_no_private" : "remote.a_phone_on_the_same") }}</span>
        </span>
        <Switch data-action="share.toggle" :on="st.open" :busy="busy || noNetwork" :label="t('remote.allow_phones_to_connect')" :on-click="toggle" />
      </div>

      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("remote.remember_paired_phones") }}</span>
          <span class="ds">{{ t("remote.when_on_a_restart_reopens") }}</span>
        </span>
        <Switch data-action="share.remember" :on="remember ?? false" :busy="busy || remember === null" :label="t('remote.remember_paired_phones')" :on-click="saveRemember" />
      </div>

      <label v-if="st.addresses.length > 1" class="rmtf">
        <span>{{ t("common.network") }}</span>
        <select data-action="share.address" :value="chosen" :disabled="busy" @change="pick(($event.target as HTMLSelectElement).value)">
          <option v-for="a in st.addresses" :key="a.ip" :value="a.ip">
            {{ a.kind === "tailnet" ? `${a.ip} · Tailscale` : a.kind === "virtual" ? `${a.ip} · ${a.interface} · ${t("remote.virtual_adapter")}` : `${a.ip} · ${a.interface}` }}
          </option>
        </select>
      </label>
      <p v-if="st.addresses.length > 1 && st.open && st.devices.length > 0" class="rmthint">{{ t("remote.switching_networks_disconnects_the_connected") }}</p>

      <label class="rmtf">
        <span>{{ t("common.port") }}</span>
        <input
          data-action="share.port"
          inputmode="numeric"
          maxlength="5"
          :aria-label="t('common.port')"
          :value="portDraft ?? (st.port ? String(st.port) : '')"
          :placeholder="t('common.auto')"
          :disabled="busy"
          @input="portDraft = ($event.target as HTMLInputElement).value.replace(/\D/g, '')"
          @blur="commitPort"
        />
      </label>
      <p class="rmthint">{{ t("remote.leave_empty_to_let_the") }}</p>
      <p v-if="st.open && st.devices.length > 0" class="rmthint">{{ t("remote.changing_the_port_disconnects_the") }}</p>

      <div v-if="st.open" class="shareopen">
        <div v-if="offer" class="shareqr">
          <img :src="`data:image/svg+xml;charset=utf-8,${encodeURIComponent(offer.qr)}`" :alt="t('remote.pairing_qr_code')" width="184" height="184" />
          <div class="sharehow">
            <span class="lb">{{ t("remote.scan_with_your_phones_camera") }}</span>
            <span class="ds">{{ t("remote.the_code_pairs_one_phone", { time: clock(offer.expires) }) }}</span>
            <code dir="ltr">{{ origin }}</code>
            <button type="button" class="rmtlnk sharecopy" data-action="share.copy" @click="copyLink(offer.url)">
              {{ t(copied === "done" ? "common.copied" : copied === "failed" ? "remote.copy_blocked" : "remote.copy_pairing_link") }}
            </button>
          </div>
        </div>
        <button v-else type="button" class="rmtadd" data-action="share.offer" :disabled="busy" @click="newCode">{{ t("remote.show_a_pairing_code") }}</button>

        <div class="sharedevs">
          <div v-for="(d, i) in st.devices" :key="d.id" class="sharedev">
            <span class="nm" :title="d.name">{{ deviceLabel(i) }}</span>
            <span class="st">{{ seenSaid(d) }}</span>
            <span v-if="confirm === d.id" class="rmtconfirm" role="alertdialog">
              <span>{{ t("remote.disconnect_this_phone_it_will") }}</span>
              <button type="button" data-action="share.keep" :data-target="d.id" @click="confirm = ''">{{ t("common.cancel") }}</button>
              <button type="button" data-danger="" data-action="share.revoke" :data-target="d.id" @click="confirm = ''; revoke(d.id)">{{ t("remote.disconnect") }}</button>
            </span>
            <button v-else type="button" class="rmtlnk" data-danger="" data-action="share.ask-revoke" :data-target="d.id" @click="confirm = d.id">{{ t("remote.disconnect") }}</button>
          </div>
          <p v-if="st.devices.length === 0" class="rmtempty">{{ t("remote.no_phone_is_connected_yet") }}</p>
        </div>

        <p class="sharewarn">{{ t("remote.the_connection_is_plain_http") }}</p>
      </div>
    </div>
  </Group>

  <Group id="relay" :title="t('relay.web_access')">
    <div class="share">
      <div v-if="relay" class="lrow">
        <i v-if="relayOn" class="pstate" :data-state="relay.online ? 'use' : 'warn'" aria-hidden="true" />
        <span class="tx">
          <span class="ds">{{ t(relayOn ? (relay.online ? "relay.online" : "relay.offline") : "relay.web_access_hint") }}</span>
        </span>
        <code v-if="relayOn" class="sc" dir="ltr" :title="relay.deviceId">{{ shortDevice }}</code>
        <button v-if="!relayOn" type="button" class="btn sm" data-action="relay.enable" :disabled="relayBusy" @click="enableRelay">{{ t("relay.turn_on") }}</button>
        <template v-else>
          <button type="button" class="btn sm" data-action="relay.pair" :disabled="relayBusy" @click="showCode">{{ t("relay.show_code") }}</button>
          <button type="button" class="btn sm" data-action="relay.disable" :disabled="relayBusy" @click="disableRelay">{{ t("relay.turn_off") }}</button>
        </template>
      </div>
      <div v-if="pairing" class="sharehow" data-relay-code>
        <span class="lb">{{ t("relay.code_label") }}</span>
        <code dir="ltr">{{ pairing.code }}</code>
        <span class="ds">{{ t("relay.code_expires", { n: relayMinutes }) }}</span>
        <span class="ds">{{ t("relay.web_access_hint") }}</span>
      </div>
    </div>
  </Group>
</template>
