<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { StudioLogs } from "../../../port/version";
import CopyButton from "../../CopyButton.vue";

// Where to look when something goes wrong: the folder the shell and the kernel
// write to, and any crash file left after an abnormal exit.
const props = defineProps<{ port: Pick<AgentPort, "studioLogs"> }>();

const logs = ref<StudioLogs | null>(null);
const failed = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.studioLogs().then((l) => { if (live) logs.value = l; }, (e) => { if (live) failed.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

const at = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
};
</script>

<template>
  <p v-if="failed" class="acct-note" role="alert">{{ failed }}</p>
  <p v-else-if="!logs" class="acct-note">{{ t("common.loading") }}</p>
  <div v-else class="vers">
    <p class="acct-note">
      <code>{{ logs.dir }}</code>
      <CopyButton :text="logs.dir" icon-only show-feedback :label="t('data.copy_the_log_folder')" />
    </p>
    <p v-if="logs.crashes.length === 0" class="acct-note">{{ t("data.no_abnormal_exit_record_has") }}</p>
    <template v-else>
      <p class="acct-note">{{ t("data.records_left_when_the_kernel") }}</p>
      <p v-for="c in logs.crashes" :key="c.name" class="acct-note">
        <code>{{ c.name }}</code> · {{ at(c.at) }} · {{ Math.max(1, Math.round(c.bytes / 1024)) }} KB
      </p>
      <p v-if="logs.truncated" class="acct-note">{{ t("data.older_records_are_not_listed") }}</p>
    </template>
  </div>
</template>
