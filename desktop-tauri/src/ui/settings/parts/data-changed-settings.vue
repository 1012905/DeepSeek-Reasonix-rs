<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { ChangedSettings } from "../../../port/boundary";

// Which keys this machine has changed from the defaults, and where they live.
// It is the answer to "why is it behaving differently from a fresh install", and
// every changed key is still a real config entry — nothing here is written.
const props = defineProps<{ port: AgentPort }>();

const changed = ref<ChangedSettings | null>(null);
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.changedSettings().then((c) => { if (live) changed.value = c; }).catch((e) => { if (live) error.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });
</script>

<template>
  <p v-if="error" class="why" role="alert">{{ error }}</p>
  <p v-else-if="!changed" class="note">{{ t("common.loading") }}</p>
  <template v-else>
    <p class="note">
      {{ changed.keys.length > 0 ? t("settings.n_differ_from_the_defaults", { n: changed.keys.length }) : t("settings.every_setting_is_the_default") }}
    </p>
    <ul v-if="changed.keys.length > 0" class="changed-keys">
      <li v-for="key in changed.keys" :key="key">{{ key }}</li>
    </ul>
    <p class="note"><code>{{ changed.path }}</code></p>
  </template>
</template>
