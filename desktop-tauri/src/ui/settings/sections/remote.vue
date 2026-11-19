<script setup lang="ts">
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
</script>

<template>
  <SectionCard v-if="drawn" :title="t('settings.other_machines')">
    <Group id="remote" :title="t('settings.remote')" :hint="t('settings.remote_hint')">
      <RemoteHosts :hub="settingsProps.hub" :on-error="settingsProps.onError" />
    </Group>

    <Group id="phone" :title="t('remote.phone_access')" :hint="t('settings.phone_hint')">
      <RemotePhone :hub="settingsProps.hub" :on-error="settingsProps.onError" />
    </Group>
  </SectionCard>
</template>
