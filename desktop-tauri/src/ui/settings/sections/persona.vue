<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import type { AgentPort } from "../../../port/port";
import type { AgentPresetCatalog, AgentPresetVersion } from "../../../port/composition";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import SectionCard from "../../SectionCard.vue";
import { ApplyNote, ScopeTag } from "../../Group.vue";
import CompositionPickerSetting from "../../CompositionPickerSetting.vue";
import Persona from "../../Persona.vue";
import { drawsHere } from "./nesting";
import { newPreset } from "../../presetDraft";
import { setNewTaskPreset, useNewTaskPreset } from "../../../state/newTaskPreset";

// The person's own definition of who the agent is, and the presets that bind
// one: a saved preset is an immutable version a conversation keeps, so this
// page lists and copies them rather than editing them in place. Editing a
// preset happens in the full editor; what lives here is the choice of which
// one, plus the persona every conversation falls back to.
//
// Two pages draw these blocks: this one when `at` names "persona", and session,
// which persona folds into. The column holds one of them at a time; ./nesting is
// what keeps it one if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("persona");
const port = computed(() => props.settingsProps.port);

const catalog = ref<AgentPresetCatalog | null>(null);
const currentId = ref("");
const loadError = ref("");
const actionError = ref("");
const note = ref("");
const busy = ref(false);

const preferred = useNewTaskPreset();

async function load() {
  // A section that has been superseded by its home page draws nothing, so it
  // asks the kernel for nothing either.
  if (!drawn) return;
  loadError.value = "";
  try {
    const [next, binding] = await Promise.all([port.value.agentPresets(), port.value.agentComposition()]);
    catalog.value = next;
    currentId.value = binding?.id ?? "";
  } catch (e) {
    loadError.value = reason(e);
    catalog.value = null;
  }
}

watch(port, () => { catalog.value = null; void load(); }, { immediate: true });

const groups = computed(() => {
  const all = catalog.value?.presets ?? [];
  return (["builtin", "user", "project"] as const)
    .map((scope) => ({
      scope,
      label: t(scope === "builtin" ? "common.built_in" : scope === "user" ? "persona.personal" : "persona.shared_with_project"),
      items: all.filter((v) => v.definition.id.startsWith(`${scope}:`)),
    }))
    .filter((g) => g.items.length > 0);
});

const nameOf = (v: AgentPresetVersion) => (v.definition.id.startsWith("builtin:") ? t(v.definition.name) : v.definition.name);
const aboutOf = (v: AgentPresetVersion) => (v.definition.id.startsWith("builtin:") ? t(v.definition.description) : v.definition.description);

async function act(work: (p: AgentPort) => Promise<unknown>, done: string) {
  busy.value = true;
  actionError.value = "";
  note.value = "";
  try {
    await work(port.value);
    note.value = done;
    await load();
  } catch (e) {
    actionError.value = reason(e);
  } finally {
    busy.value = false;
  }
}

const makeDefault = (v: AgentPresetVersion) => {
  if (!setNewTaskPreset(v.definition.id)) actionError.value = t("persona.could_not_save_the_new");
  else note.value = t("persona.set_name_as_the_new", { name: nameOf(v) });
};

const copyPreset = (v: AgentPresetVersion) => {
  const base = catalog.value;
  if (!base) return;
  const draft = newPreset(v.definition, base, true);
  void act((p) => p.saveAgentPreset(draft, ""), t("persona.saved_new_conversations_use_the"));
};

const removePreset = (v: AgentPresetVersion) =>
  void act((p) => p.deleteAgentPreset(v.definition.id, v.revision), t("persona.preset_deleted_existing_conversations_keep"));

const onImport = async (e: Event) => {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file || !catalog.value) return;
  actionError.value = "";
  note.value = "";
  try {
    if (file.size > 4 * 1024 * 1024) throw new Error(t("ext.this_file_is_too_large"));
    const imported = await port.value.importAgentPreset(JSON.parse(await file.text()));
    const draft = newPreset(imported.definition, catalog.value, true);
    await act((p) => p.saveAgentPreset(draft, ""), t("persona.saved_new_conversations_use_the"));
  } catch (err) {
    actionError.value = reason(err);
  }
};

const diagnostics = computed(() => catalog.value?.diagnostics ?? []);
const loading = computed(() => catalog.value === null && !loadError.value);
</script>

<template>
  <SectionCard v-if="drawn" :title="t('settings.persona_and_presets')">
    <section id="set-agent-presets" data-setting="agent-presets" class="grp preset-workspace">
      <div class="grp-hd">
        <h4>{{ t("persona.agent_presets") }}</h4>
        <ScopeTag id="agent-presets" />
      </div>
      <p class="hint">{{ t("persona.presets_define_which_capabilities_are") }}</p>

      <div v-if="loadError" class="why" role="alert">
        <p>{{ t("persona.agent_presets_unavailable") }}</p>
        <p>{{ loadError }}</p>
        <button class="act" type="button" data-action="agent-preset.reload" @click="load">{{ t("common.retry") }}</button>
      </div>
      <p v-else-if="loading" class="empty">{{ t("persona.loading_presets") }}</p>
      <div v-else class="preset-list">
        <CompositionPickerSetting />
        <p class="ds">{{ t("persona.click_a_card_to_set") }}</p>

        <section v-for="g in groups" :key="g.scope" class="composition-list-group" :aria-label="g.label">
          <h3>{{ g.label }}</h3>
          <article v-for="v in g.items" :key="v.definition.id" class="composition-row" :data-current="currentId === v.definition.id ? '' : undefined">
            <span class="composition-row-copy">
              <strong>
                {{ nameOf(v) }}
                <small v-if="v.definition.id === 'builtin:general'">{{ t("common.default") }}</small>
                <small v-if="preferred === v.definition.id">{{ t("persona.new_task_default") }}</small>
                <small v-if="currentId === v.definition.id">{{ t("common.this_session") }}</small>
              </strong>
              <span>{{ aboutOf(v) }}</span>
            </span>
            <span class="composition-toolbar">
              <button v-if="g.scope !== 'project'" class="act" type="button" data-action="agent-preset.new-task-default"
                :data-target="v.definition.id" :aria-pressed="preferred === v.definition.id"
                :aria-label="t('persona.set_name_as_the_new', { name: nameOf(v) })"
                :disabled="busy || preferred === v.definition.id" @click="makeDefault(v)">
                {{ t(preferred === v.definition.id ? "persona.new_task_default" : "common.make_default") }}
              </button>
              <button class="act" type="button" data-action="agent-preset.copy" :data-target="v.definition.id"
                :aria-label="t('persona.duplicate_name', { name: nameOf(v) })" :disabled="busy" @click="copyPreset(v)">
                {{ t("common.copy") }}
              </button>
              <button v-if="g.scope !== 'builtin'" class="act ghost" type="button" data-action="agent-preset.delete"
                :data-target="v.definition.id" :disabled="busy" @click="removePreset(v)">
                {{ t("common.delete") }}
              </button>
            </span>
          </article>
        </section>

        <div class="composition-toolbar">
          <label class="act preset-import">
            {{ t("persona.import_a_preset_file_preview") }}
            <input type="file" accept="application/json,.json" :aria-label="t('persona.import_a_preset_file_preview')"
              data-action-change="agent-preset.import" @change="onImport" />
          </label>
          <button class="act" type="button" data-action="agent-preset.reload" :disabled="busy" @click="load">{{ t("common.reload") }}</button>
        </div>

        <p v-for="(d, i) in diagnostics" :key="i" class="ds">{{ `${d.module ?? ""}: ${d.detail}` }}</p>
        <p v-if="note" class="find" role="status">{{ note }}</p>
        <p v-if="actionError" class="find" data-lvl="err" role="alert">{{ actionError }}</p>
        <p class="ds">{{ t("persona.available_to_new_conversations_after") }}</p>
        <ApplyNote id="agent-presets" />
      </div>
    </section>

    <!-- The persona block is a sub-region of the presets in the table of
         contents: it is the fallback every preset inherits. -->
    <Persona :port="settingsProps.port" :on-saved="settingsProps.onChanged" />
  </SectionCard>
</template>
