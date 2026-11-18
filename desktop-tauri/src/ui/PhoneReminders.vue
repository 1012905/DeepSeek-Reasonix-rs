<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import { accountOrigin, ownFetch } from "../port/cloud_remote";
import { AccountError, accountClient, indexedDbKeyStore, uaClassOf } from "../port/phone_device";
import { EnableRefused, realPushBrowser, type PushCategories } from "../port/phone_push";
import { phoneReminders, type PhoneReminders, type Reminders } from "../port/phone_reminders";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

function realReminders(host: string): PhoneReminders {
  return phoneReminders(host, {
    account: accountClient(accountOrigin, ownFetch),
    keys: indexedDbKeyStore,
    browser: realPushBrowser(),
    uaClass: uaClassOf(navigator.userAgent),
  });
}

function problemFor(error: unknown): string {
  if (error instanceof EnableRefused) {
    if (error.reason === "denied") return t("remote.the_browser_did_not_allow");
    if (error.reason === "unsupported") return t("remote.this_browser_cannot_receive_reminders");
    if (error.reason === "push_service") return t("remote.the_browser_could_not_register");
    return t("remote.this_phone_has_not_been");
  }
  if (error instanceof AccountError && (error.status === 401 || error.code === "remote_reauth_required")) {
    return t("remote.sign_in_to_your_account");
  }
  return t("remote.the_account_service_is_unavailable");
}

/** What this phone does about reminders from one computer. Switching on is what
 *  registers the phone with the account: reading this block creates nothing. */
const props = defineProps<{ host: string; reminders?: PhoneReminders }>();

const live = computed(() => props.reminders ?? realReminders(props.host));
const state = ref<Reminders | null>(null);
const busy = ref(false);
const problem = ref("");

watch(live, (live) => {
  let current = true;
  live.load().then((s) => { if (current) state.value = s; }).catch((e) => { if (current) { state.value = { kind: "off" }; problem.value = problemFor(e); } });
  return () => { current = false; };
}, { immediate: true });

// The first call into `live` stays in the press that made it: the browser's
// permission prompt is only shown from one.
function run(go: () => Promise<Reminders>) {
  busy.value = true;
  problem.value = "";
  go().then((s) => { state.value = s; }).catch((e) => { problem.value = problemFor(e); }).finally(() => { busy.value = false; });
}

const supported = computed(() => live.value.supported());
const on = computed(() => state.value?.kind === "on" ? state.value : null);
function choose(patch: Partial<PushCategories>) {
  const cur = on.value;
  if (cur) run(() => live.value.choose({ ...cur.status.categories, ...patch }));
}
</script>

<template>
  <Group id="phone-device" :title="t('remote.this_phones_reminders')"
    :hint="t('remote.when_something_on_your_computer')">
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("remote.remind_this_phone") }}</span>
        <span class="ds">{{ t("remote.the_first_time_you_switch") }}</span>
      </span>
      <Switch data-action="phone-device.enabled" :on="!!on" :busy="busy || state === null || !supported" :label="t('remote.remind_this_phone')"
        :on-click="() => run(on ? live.disable : live.enable)" />
    </div>
    <div v-if="!supported" class="peernote">{{ t("remote.this_browser_cannot_receive_notifications") }}</div>
    <div v-if="state?.kind === 'waiting'" class="peernote">
      {{ t("remote.this_phone_is_waiting_for") }}
      <button class="act" data-action="phone-device.retry" :disabled="busy" @click="run(live.enable)">{{ t("remote.try_again") }}</button>
    </div>
    <div v-if="state?.kind === 'removed'" class="peernote">
      {{ t("remote.the_computer_has_removed_this") }}
      <button class="act" data-action="phone-device.restart" :disabled="busy" @click="run(live.startOver)">{{ t("remote.register_again") }}</button>
    </div>
    <template v-if="on">
      <div class="lrow subrow">
        <span class="tx"><span class="lb">{{ t("remote.an_action_needs_approval") }}</span><span class="ds">{{ t("remote.the_model_wants_to_touch") }}</span></span>
        <Switch data-action="phone-device.occasion" data-value="approval" :on="on?.status.categories.approval ?? false" :busy="busy"
          :label="t('remote.an_action_needs_approval')" :on-click="() => choose({ approval: !on?.status.categories.approval })" />
      </div>
      <div class="lrow subrow">
        <span class="tx"><span class="lb">{{ t("remote.the_model_asks_me_something") }}</span><span class="ds">{{ t("remote.the_model_has_stopped_and") }}</span></span>
        <Switch data-action="phone-device.occasion" data-value="input" :on="on?.status.categories.input ?? false" :busy="busy"
          :label="t('remote.the_model_asks_me_something')" :on-click="() => choose({ input: !on?.status.categories.input })" />
      </div>
      <div class="lrow subrow">
        <span class="tx"><span class="lb">{{ t("remote.a_turn_ends_in_an") }}</span><span class="ds">{{ t("remote.the_turn_ended_in_an") }}</span></span>
        <Switch data-action="phone-device.occasion" data-value="error" :on="on?.status.categories.error ?? false" :busy="busy"
          :label="t('remote.a_turn_ends_in_an')" :on-click="() => choose({ error: !on?.status.categories.error })" />
      </div>
      <div v-if="on.status.failureCount > 0" class="peernote" role="status">
        {{ on.status.lastFailureStatus
          ? t("remote.the_last_reminder_was_not_2", { status: on.status.lastFailureStatus })
          : t("remote.the_last_reminder_was_not") }}
      </div>
    </template>
    <div v-if="problem" class="why" role="alert">{{ problem }}</div>
  </Group>
</template>