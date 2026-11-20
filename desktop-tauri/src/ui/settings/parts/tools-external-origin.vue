<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, savedNotApplied } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import Switch from "../../Switch.vue";
import SaveNote from "../../SaveNote.vue";
import type { SaveNoteState } from "../../SaveNote.vue";

// One switch over asking before a call copies text an external result gave the
// task. The save sends the lists as the kernel holds them now, because the rules
// editor saves the same record and a stale copy would overwrite it.
const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const on = ref<boolean | null>(null);
const busy = ref(false);
const note = ref<SaveNoteState | null>(null);

watch(() => props.port, (port) => {
  let live = true;
  on.value = null;
  note.value = null;
  port.permissions()
    .then((p) => { if (live) on.value = p.askOnExternalOrigin === true; })
    .catch(() => { if (live) on.value = null; });
  return () => { live = false; };
}, { immediate: true });

const flip = async () => {
  if (on.value === null) return;
  busy.value = true;
  note.value = null;
  try {
    const { mode, deny, ask, allow } = await props.port.permissions();
    const saved = await props.port.savePermissions({ mode, deny, ask, allow, askOnExternalOrigin: !on.value });
    on.value = saved.askOnExternalOrigin === true;
    props.onChanged();
  } catch (e) {
    note.value = { text: reason(e), unapplied: savedNotApplied(e) };
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div v-if="on === null" class="empty">{{ t("safety.this_setting_could_not_be") }}</div>
  <div v-else>
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("safety.ask_before_copied_text") }}</span>
        <span class="ds">{{ t("safety.what_outside_text_covers") }}</span>
      </span>
      <Switch
        data-action="permissions.ask-on-external-origin"
        :on="on"
        :busy="busy"
        :label="t('settings.outside_text')"
        :on-click="() => void flip()"
      />
    </div>
    <SaveNote :note="note" />
  </div>
</template>
