<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import Account from "../../Account.vue";
import VersionsSection from "./versions.vue";
import { declarePage } from "./nesting";
import TraceExport from "../parts/data-trace-export.vue";

// The account Reasonix needs only for the networked surfaces — the forum and
// crash follow-up. Signing in uploads no conversation, code or key, and the
// trace export beside it is the other thing that leaves this machine on purpose.
//
// Versions log and community fold into this page (FOLDS_INTO), so they are
// drawn from their own file rather than copied here: the index under this
// page's title promises their anchors, and one implementation is what keeps the
// page and the version page's own spelling from drifting apart.
const props = defineProps<{ settingsProps: SettingsProps }>();
declarePage("account");

const port = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
</script>

<template>
  <SectionCard :title="t('settings.account')">
    <Group id="account" :title="t('settings.account')" :hint="t('settings.account_hint')">
      <Account
        :port="port"
        :state="settingsProps.account"
        :unread="settingsProps.accountUnread"
        :reload="settingsProps.reloadAccount"
      />
    </Group>

    <!-- Nothing here is sent unless the person names a collector and switches it
         on, which is why the block can sit on the same page as the sign-in. -->
    <TraceExport :port="props.settingsProps.port" />
  </SectionCard>

  <!-- What is running, its logs, and who to thank: a page of its own as well as
       blocks on this one, drawn from the file that answers for "versions". -->
  <VersionsSection :settings-props="settingsProps" />
</template>
