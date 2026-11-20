<script setup lang="ts">
import { computed, ref } from "vue";
import type { AgentPort } from "../port/port";
import type { AgentPresetVersion } from "../port/composition";
import { t } from "../i18n";

const props = defineProps<{
  port: AgentPort;
  version: AgentPresetVersion;
  busy: boolean;
  run: (work: () => Promise<void>) => Promise<void>;
  onDeleted: () => void;
  onPersona: () => void;
}>();

const deleting = ref(false);
const exported = ref("");
const note = ref("");
const d = computed(() => props.version.definition);

const exportPreset = () => void props.run(async () => {
  exported.value = JSON.stringify(await props.port.exportAgentPreset(d.value.id), null, 2);
});
const deletePreset = () => void props.run(async () => {
  await props.port.deleteAgentPreset(d.value.id, props.version.revision);
  props.onDeleted();
});
const setDefault = (workspace: boolean) => void props.run(async () => {
  await props.port.setDefaultAgentPreset(d.value.id, workspace);
  note.value = t(workspace ? "persona.set_as_workspace_preference" : "persona.set_as_personal_preference");
});
</script>

<template>
  <details class="composition-extra">
    <summary>{{ t("common.more_actions") }}</summary>
    <button v-if="d.id === 'builtin:general'" type="button" class="act" data-action="agent-preset.persona" :disabled="busy" @click="onPersona">{{ t("persona.edit_everyday_partner_persona") }}</button>
    <div class="composition-toolbar">
      <button type="button" class="act" data-action="agent-preset.export" :disabled="busy" @click="exportPreset">{{ t("persona.export_file") }}</button>
      <button v-if="!d.id.startsWith('builtin:')" type="button" class="act" data-action="agent-preset.delete-preview" :disabled="busy" @click="deleting = true">{{ t("persona.delete_preset") }}</button>
    </div>
    <a v-if="exported" class="act" download="agent-preset.json"
      :href="`data:application/json;charset=utf-8,${encodeURIComponent(exported)}`">{{ t("persona.download_preset_json") }}</a>
    <div v-if="deleting" class="find" role="group" :aria-label="t('persona.delete_preset')">
      <p>{{ t("persona.deleting_removes_it_from_new") }}</p>
      <button type="button" class="act" data-action="agent-preset.delete" :disabled="busy" @click="deletePreset">{{ t("common.confirm_delete") }}</button>
      <button type="button" class="act" data-action="agent-preset.delete-cancel" :disabled="busy" @click="deleting = false">{{ t("persona.cancel_deletion") }}</button>
    </div>
    <details>
      <summary>{{ t("persona.explicit_default_configuration") }}</summary>
      <p class="ds">{{ t("persona.used_only_when_explicitly_selecting") }}</p>
      <div class="composition-toolbar">
        <button type="button" class="act" data-action="agent-preset.default" :disabled="busy || d.id.startsWith('project:')"
          @click="setDefault(false)">{{ t("persona.set_personal_preference") }}</button>
        <button type="button" class="act" data-action="agent-preset.workspace-default" :disabled="busy"
          @click="setDefault(true)">{{ t("persona.set_workspace_preference") }}</button>
      </div>
      <p v-if="note" role="status">{{ note }}</p>
    </details>
  </details>
</template>