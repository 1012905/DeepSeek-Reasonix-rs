<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AccountState, AgentPort } from "../port/port";

// App owns the account read so the panel and sidebar show the same state.
const props = defineProps<{ port: AgentPort; state: AccountState | null; unread?: string; reload: () => void }>();

const code = ref<{ userCode: string; uri: string } | null>(null);
const busy = ref(false);
const confirm = ref(false);
const error = ref("");
const cancelled = ref(false);

const signIn = async () => {
  error.value = "";
  busy.value = true;
  cancelled.value = false;
  try {
    const grant = await props.port.accountLogin();
    const uri = grant.verificationUriComplete || grant.verificationUri;
    code.value = { userCode: grant.userCode, uri };
    // The approval page opens in the real browser: a window with no address
    // bar cannot be checked, and the password manager lives there anyway.
    await props.port.openExternal(uri);
    const deadline = Date.now() + grant.expiresIn * 1000;
    let wait = Math.max(grant.interval, 1) * 1000;
    while (!cancelled.value && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, wait));
      if (cancelled.value) return;
      const res = await props.port.accountPoll(grant.deviceCode);
      if (res.status === "complete") {
        code.value = null;
        props.reload();
        return;
      }
      if (res.slowDown) wait += 5000;
    }
    if (!cancelled.value) error.value = t("settings.this_sign_in_request_expired");
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
    code.value = null;
  }
};

const signOut = async () => {
  // Confirmation protects a click aimed at sign-in before state refreshes.
  if (!confirm.value) {
    confirm.value = true;
    return;
  }
  confirm.value = false;
  busy.value = true;
  try {
    await props.port.accountLogout();
    props.reload();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div v-if="unread" class="find" data-lvl="warn" role="alert">
    <span class="t">{{ t("settings.sign_in_status_could_not") }}</span>
    <span class="why">
      {{ unread }}
      <button class="lnk" data-action="account.reload" @click="reload">{{ t("common.retry") }}</button>
    </span>
  </div>

  <p v-else-if="state === null" class="acct-note">{{ t("settings.checking_sign_in_status") }}</p>

  <div v-else-if="state.signedIn" class="acct">
    <div class="acct-who">
      <span class="nm">{{ state.user?.label || state.user?.handle || state.user?.email || t("settings.signed_in") }}</span>
      <span v-if="state.user?.email" class="em">{{ state.user.email }}</span>
    </div>
    <p v-if="state.error" class="acct-note" role="alert">
      {{ t("settings.cannot_reach_the_identity_service") }}：{{ state.error }}{{ " " }}
      <button class="lnk" data-action="account.reload" :disabled="busy" @click="reload">{{ t("common.retry") }}</button>
    </p>
    <div class="acct-act">
      <button class="btn" data-action="account.sign-out" :disabled="busy" @click="signOut" @mouseleave="confirm = false">{{ t(confirm ? "settings.confirm_sign_out" : "settings.sign_out") }}</button>
      <span class="acct-note">{{ t("settings.clears_the_local_credential_only") }}</span>
    </div>
    <p v-if="error" class="acct-note" data-err="">{{ error }}</p>
  </div>

  <div v-else class="acct">
    <p class="acct-note">{{ t("settings.used_for_posting_to_the") }}</p>
    <p v-if="state.expired" class="acct-note">{{ t("settings.the_previous_sign_in_has") }}</p>
    <div v-if="code" class="acct-code">
      <span class="cd">{{ code.userCode }}</span>
      <span class="acct-note">{{ t("settings.type_this_code_into_the", { uri: code.uri }) }}</span>
      <button class="btn" @click="cancelled = true">{{ t("common.cancel") }}</button>
    </div>
    <div v-else class="acct-act">
      <button class="btn" data-action="account.sign-in" :disabled="busy" @click="signIn">{{ t(busy ? "settings.opening" : "common.sign_in") }}</button>
    </div>
    <p v-if="error" class="acct-note" data-err="">{{ error }}</p>
  </div>
</template>