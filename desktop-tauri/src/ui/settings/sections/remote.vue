<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import { drawsHere } from "./nesting";
import RemoteHosts from "../parts/data-remote-hosts.vue";
import RemotePhone from "../parts/data-remote-phone.vue";

// Other machines, and the phones that reach this one. The host book belongs to
// the window rather than to a pane, so the hub answers it and not the port the
// active session runs on.
//
// Two pages draw these blocks: this one when `at` names "remote", and network,
// which remote folds into. The column holds one of them at a time; ./nesting is
// what keeps it one if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("remote");

// The phone and web-access blocks are answered by this machine's own kernel,
// so they read the window's port even while a pane is on another machine.
const port = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
</script>

<template>
  <SectionCard v-if="drawn" :title="t('settings.other_machines')">
    <Group id="remote" :title="t('settings.remote')" :hint="t('settings.remote_hint')">
      <RemoteHosts :hub="settingsProps.hub" :on-error="settingsProps.onError" />
    </Group>

    <RemotePhone :hub="settingsProps.hub" :port="port" :on-error="settingsProps.onError" />
  </SectionCard>
</template>
