<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { HttpError } from "../port/http_error";
import type { AgentPort, NotifyPrefs, PhoneNotifyPrefs } from "../port/port";
import type { HubPort } from "../port/hub";
import type { PhoneView } from "../port/share";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const POLL_MS = 10_000;

const when = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
};

/** What this computer tells the phones that have signed in to the same account.
 *  Only the kind of occasion and a count leave this machine: no session name,
 *  command or text. A phone chooses on its own page whether to receive them. */
const props = defineProps<{ hub: HubPort; port: AgentPort; onError: (e: unknown) => void }>();

const prefs = ref<NotifyPrefs | null>(null);
const phones = ref<PhoneView[] | null>(null);
const problem = ref("");
const busy = ref("");
const confirm = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.notifyPrefs().then((p) => { if (live) prefs.value = p; }).catch(() => { if (live) prefs.value = null; });
  return () => { live = false; };
}, { immediate: true });

async function readPhones() {
  try {
    const list = await props.hub.sharePhones();
    phones.value = list ? list.phones : null;
    problem.value = "";
  } catch (e) {
    problem.value = reason(e);
    if (e instanceof HttpError && e.reason?.code === "phones.unavailable") phones.value = null;
  }
}

watch(() => props.hub, () => {
  void readPhones();
  const timer = setInterval(() => void readPhones(), POLL_MS);
  return () => clearInterval(timer);
}, { immediate: true });

function flip(patch: Partial<PhoneNotifyPrefs>) {
  if (!prefs.value) return;
  props.port.setNotifyPrefs({ ...prefs.value, phone: { ...prefs.value.phone, ...patch } })
    .then((got) => { if (got) prefs.value = got; })
    .catch(props.onError);
}

async function act(id: string, fn: () => Promise<{ phones: PhoneView[] }>) {
  busy.value = id;
  try {
    phones.value = (await fn()).phones;
    problem.value = "";
    confirm.value = "";
  } catch (e) {
    problem.value = reason(e);
  } finally {
    busy.value = "";
  }
}

// A kernel with no window answers no preferences at all, and a settings page
// does not grow a heading for a switch it cannot show.
const on = computed(() => prefs.value?.phone.enabled ?? false);
</script>

<template>
  <Group v-if="prefs" id="phone-notify" :title="t('remote.phone_notifications')"
    :hint="t('remote.when_something_is_waiting_for')">
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("remote.remind_my_phones") }}</span>
        <span class="ds">{{ t("remote.when_off_this_computer_reports") }}</span>
      </span>
      <Switch data-action="phone-notify.enabled" :on="on" :label="t('remote.remind_my_phones')" :on-click="() => flip({ enabled: !on })" />
    </div>
    <div class="lrow subrow" :data-off="on ? undefined : ''">
      <span class="tx">
        <span class="lb">{{ t("common.when_something_waits_for_approval") }}</span>
        <span class="ds">{{ t("common.the_model_wants_to_touch") }}</span>
      </span>
      <Switch data-action="phone-notify.approval" :on="prefs.phone.approval" :busy="!on" :label="t('common.when_something_waits_for_approval')" :on-click="() => flip({ approval: !prefs!.phone.approval })" />
    </div>
    <div class="lrow subrow" :data-off="on ? undefined : ''">
      <span class="tx">
        <span class="lb">{{ t("common.when_the_model_asks_you") }}</span>
        <span class="ds">{{ t("common.the_model_stopped_to_ask") }}</span>
      </span>
      <Switch data-action="phone-notify.input" :on="prefs.phone.input" :busy="!on" :label="t('common.when_the_model_asks_you')" :on-click="() => flip({ input: !prefs!.phone.input })" />
    </div>
    <div class="lrow subrow" :data-off="on ? undefined : ''">
      <span class="tx">
        <span class="lb">{{ t("remote.when_a_turn_fails") }}</span>
        <span class="ds">{{ t("remote.the_turn_ended_in_an") }}</span>
      </span>
      <Switch data-action="phone-notify.error" :on="prefs.phone.error" :busy="!on" :label="t('remote.when_a_turn_fails')" :on-click="() => flip({ error: !prefs!.phone.error })" />
    </div>

    <template v-if="phones">
      <div class="peernote"><b>{{ t("remote.enrolled_phones") }}</b></div>
      <div v-if="phones.length === 0" class="peernote">{{ t("remote.no_phone_is_enrolled_yet") }}</div>
      <ul class="wa-list" :aria-label="t('remote.enrolled_phones')">
        <li v-for="p in phones" :key="p.id">
          <span class="tx">
            <span class="lb">{{ p.name }}{{ p.ordinal ? ` · ${t("remote.device_n", { n: p.ordinal })}` : "" }}</span>
            <span class="ds">
              {{ p.state === "pending" ? t("remote.waiting_for_you_fingerprint_id", { id: p.keyThumbprint }) : t("remote.last_used_at", { at: when(p.lastSeenAt) }) }}
            </span>
          </span>
          <span class="acts">
            <template v-if="p.state === 'pending'">
              <button class="act" data-action="phone.add" :data-target="p.id" :disabled="busy === p.id" @click="act(p.id, () => hub.resolvePhone(p.id, 'add'))">{{ t("common.add") }}</button>
              <button v-if="p.replaceCandidate" class="act" data-action="phone.replace" :data-target="p.id" :disabled="busy === p.id" @click="act(p.id, () => hub.resolvePhone(p.id, 'replace', p.replaceCandidate))">{{ t("remote.replace_the_old_device") }}</button>
              <button class="act ghost" data-action="phone.reject" :data-target="p.id" :disabled="busy === p.id" @click="act(p.id, () => hub.resolvePhone(p.id, 'reject'))">{{ t("common.refuse") }}</button>
            </template>
            <button v-else-if="confirm === p.id" class="act danger" data-action="phone.revoke" :data-target="p.id" :disabled="busy === p.id" @click="act(p.id, () => hub.revokePhone(p.id))">{{ t("remote.confirm_removal") }}</button>
            <button v-else class="act ghost" data-action="phone.revoke-ask" :data-target="p.id" @click="confirm = p.id">{{ t("common.remove") }}</button>
          </span>
        </li>
      </ul>
    </template>
    <div v-if="problem" class="why" role="alert">{{ problem }}</div>
  </Group>
</template>