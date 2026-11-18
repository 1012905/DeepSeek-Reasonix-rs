<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import { remoteDeviceId } from "../../../port/cloud_remote";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import { drawsHere } from "./nesting";
import RemoteHosts from "../parts/data-remote-hosts.vue";
import RemotePhone from "../parts/data-remote-phone.vue";
import PhoneNotifications from "../../PhoneNotifications.vue";
import PhoneReminders from "../../PhoneReminders.vue";

// Other machines, and the phones that reach this one. The host book belongs to
// the window rather than to a pane, so the hub answers it and not the port the
// active session runs on.
//
// Two pages draw these blocks: this one when `at` names "remote", and network,
// which remote folds into. The column holds one of them at a time; ./nesting is
// what keeps it one if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("remote");

const port = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
// A paired device is a phone looking at this computer, so the block that asks it
// to receive reminders is only drawn there. It is read once: the query string
// cannot change while the page is open.
const device = ref(remoteDeviceId());
</script>

<template>
  <SectionCard v-if="drawn" :title="t('settings.other_machines')">
    <Group id="remote" :title="t('settings.remote')" :hint="t('settings.remote_hint')">
      <RemoteHosts :hub="settingsProps.hub" :on-error="settingsProps.onError" />
    </Group>

    <Group id="phone" :title="t('remote.phone_access')" :hint="t('settings.phone_hint')">
      <RemotePhone :hub="settingsProps.hub" :on-error="settingsProps.onError" />
      <PhoneNotifications :hub="settingsProps.hub" :port="port" :on-error="settingsProps.onError" />
      <!-- The pair below is drawn from the same two components the phone's own
           page uses, so the desktop and the phone agree about what is enrolled. -->
      <PhoneReminders v-if="device" :host="device" />
    </Group>
  </SectionCard>
</template>
