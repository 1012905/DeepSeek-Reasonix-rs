<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import RemoteSection from "./remote.vue";
import { declarePage } from "./nesting";
import Proxy from "../parts/data-network-proxy.vue";

// How the kernel reaches the outside, and which other machines and phones
// reach this one. The proxy is a machine fact — the window's own port answers
// it even while a pane runs on another machine, which is why the network port
// is separate from the active pane's.
//
// Remote machines and the phone fold into this page (FOLDS_INTO), so they are
// drawn from their own file rather than copied here: the index under this
// page's title promises their anchors, and one implementation is what keeps
// them reachable from a link that names "remote" directly.
const props = defineProps<{ settingsProps: SettingsProps }>();
declarePage("network");

const port = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
</script>

<template>
  <SectionCard :title="t('common.network')">
    <Group id="network" :title="t('common.network')" :hint="t('settings.network_hint')">
      <Proxy :port="port" :host="settingsProps.networkHost ?? ''" />
    </Group>
  </SectionCard>

  <RemoteSection :settings-props="settingsProps" />
</template>
