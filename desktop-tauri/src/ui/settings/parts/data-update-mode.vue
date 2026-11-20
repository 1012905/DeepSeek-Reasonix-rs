<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { UpdateMode } from "../../../port/version";
import { refreshUpdateWatch } from "../../../state/updatewatch";
import SettingRow from "../../SettingRow.vue";

// What this machine does about new releases on its own. Neither mode installs
// anything: notify only reads the catalog and says when a release is newer, and
// download fetches one but still waits for the person to restart into it.
//
// This is the one row for the setting, and it draws its own anchor: the
// catalogue files it under the common section (prefsnav's `update-mode`), so a
// second copy on the version page would make a search hit pick a coin toss
// between two `set-update-mode` blocks. The version page keeps the reads and
// downloads it governs; the choice itself lives here.
const MODES: readonly (readonly [UpdateMode, string])[] = [
  ["manual", "about.manual"],
  ["notify", "about.notify"],
  ["download", "about.download"],
];

const props = defineProps<{ port: AgentPort }>();

const mode = ref<UpdateMode | null>(null);
const busy = ref(false);
const error = ref("");

// Null is a kernel that cannot say how it treats new releases: the mode lives in
// the user's file and no project can change it, so there is no row to offer.
watch(() => props.port, (port) => {
  let live = true;
  port.updateMode()
    .then((m) => { if (live) mode.value = m; })
    .catch(() => { if (live) mode.value = null; });
  return () => { live = false; };
}, { immediate: true });

async function pick(next: UpdateMode): Promise<void> {
  if (!mode.value || next === mode.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    mode.value = await props.port.setUpdateMode(next);
    // The watch reads the mode again, and notify mode checks the catalog now.
    void refreshUpdateWatch();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <SettingRow
    v-if="mode"
    id="update-mode"
    :title="t('about.update_mode')"
    :desc="t('about.whether_to_check_for_and')"
  >
    <div class="seg" data-text role="radiogroup" :aria-label="t('about.update_mode')">
      <button
        v-for="[id, name] in MODES" :key="id"
        role="radio" data-action="update.mode" :data-value="id"
        :aria-checked="mode === id" :disabled="busy"
        @click="pick(id)"
      >{{ t(name) }}</button>
    </div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </SettingRow>
</template>
