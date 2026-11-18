<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import type { HubPort } from "../port/hub";
import { t } from "../i18n";
import App from "./App.vue";
import RemoteNotice from "./RemoteNotice.vue";
import { useStalePage } from "./stale_page";
import { failureView, type FailureView } from "../port/cloud_failure";
import { onRemoteConnectionEnded, openRemoteHome, remoteConnectionEnded, signInAgain, type RemoteEnd } from "../port/cloud_remote";

const props = defineProps<{ hub: HubPort; remote: string | null }>();

const ended = ref<RemoteEnd | null>(props.remote ? remoteConnectionEnded() : null);
let stop: (() => void) | undefined;
onMounted(() => {
  if (!props.remote) return;
  stop = onRemoteConnectionEnded((e) => { ended.value = e; });
});
onBeforeUnmount(() => stop?.());

const stale = useStalePage(() => !!props.remote);
const reauth = computed(() => ended.value?.kind === "reauth");
const specific = computed<FailureView | null>(() =>
  !reauth.value && ended.value?.failure ? failureView(ended.value.failure) : null);

const failureAction = (view: FailureView) => (view.action === "home" ? openRemoteHome : () => location.reload());
const failureLabel = (view: FailureView) => (view.action === "home" ? t("app.open_reasonix_io") : t("app.reconnect"));

const endedTitle = computed(() =>
  reauth.value ? t("app.sign_in_again") : specific.value?.title ?? t("app.remote_connection_ended"));
const endedBody = computed(() =>
  reauth.value ? t("app.for_security_remote_control_asks") : specific.value?.body ?? t("app.this_computer_has_stopped_web"));
const endedAction = computed(() =>
  reauth.value ? t("app.sign_in") : specific.value ? failureLabel(specific.value) : t("app.reconnect"));
const endedOnAction = () => {
  if (reauth.value) void signInAgain(props.remote!);
  else if (specific.value) failureAction(specific.value)();
  else location.reload();
};
const reload = () => location.reload();
</script>

<template>
  <App :hub="hub" />
  <RemoteNotice
    v-if="stale && !ended"
    :title="t('app.this_page_is_not_the')"
    :body="t('app.it_is_an_older_build')"
    :action="t('app.refresh_page')"
    info
    :on-action="reload"
  />
  <RemoteNotice
    v-if="ended && remote"
    :title="endedTitle"
    :body="endedBody"
    :action="endedAction"
    :wait-s="specific?.waitS"
    :retry-when-online="specific?.retryWhenOnline"
    :on-action="endedOnAction"
  />
</template>