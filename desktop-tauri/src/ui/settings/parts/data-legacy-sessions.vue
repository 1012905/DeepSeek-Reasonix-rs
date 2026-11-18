<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { LegacyImport } from "../../../port/hub";
import type { HubPort } from "../../../port/hub";

// Two doors into the same store: a 1.x data folder the kernel scans (it only
// copies, never moves or deletes), and one session file the window hands over as
// it is. Both answer with where the conversation landed, which is what a sidebar
// reload needs to show it.
const SKIP_LABEL: Record<string, string> = {
  too_large: "data.this_folder_is_too_large",
  unreadable_format: "data.this_folder_could_not_be",
  schema_unsupported: "data.this_folder_could_not_be",
  permission: "data.this_folder_could_not_be",
  corrupt: "data.this_folder_could_not_be",
  copy_failed: "data.this_folder_could_not_be",
  no_project: "data.this_folder_could_not_be",
};

const props = defineProps<{ hub: HubPort; workspace: string; onRecovered: () => void }>();

const scanning = ref(false);
const typed = ref("");
const typing = ref(false);
const recovery = ref<LegacyImport | null>(null);
const error = ref("");
const importing = ref(false);
const imported = ref("");

async function from(source: string): Promise<void> {
  scanning.value = true;
  error.value = "";
  try {
    recovery.value = await props.hub.importLegacySessions(source, props.workspace);
    typing.value = false;
    props.onRecovered();
  } catch {
    error.value = t("data.this_folder_could_not_be");
  } finally {
    scanning.value = false;
  }
}

// null is a window that has no folder picker at all; "" is one the person closed.
// Those are different answers, and a panel that conflated them asked twice.
async function pick(): Promise<void> {
  error.value = "";
  const source = await props.hub.pickFolder().catch(() => null);
  if (source === null) { typing.value = true; return; }
  if (!source) return;
  await from(source);
}

async function take(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  importing.value = true;
  error.value = "";
  imported.value = "";
  try {
    await props.hub.importNativeSession(file, props.workspace);
    imported.value = t("data.session_imported_with_its_original");
    props.onRecovered();
  } catch (e) {
    error.value = reason(e);
  } finally {
    importing.value = false;
  }
}

const submit = (e: Event) => {
  e.preventDefault();
  if (typed.value.trim()) void from(typed.value.trim());
};

watch(() => props.workspace, () => { recovery.value = null; imported.value = ""; });
</script>

<template>
  <section class="grp">
    <h3 class="lbl">{{ t("data.import_a_native_session") }}</h3>
    <div class="item">
      <div class="l">
        <b>{{ t("data.import_from_a_session_file") }}</b>
        <span class="hint">{{ t("data.keep_the_composition_version_and") }}</span>
        <span v-if="imported" class="ready">{{ imported }}</span>
      </div>
      <label class="btn" data-action="session.import-native" :data-disabled="importing || !workspace ? '' : undefined">
        {{ t(importing ? "common.importing" : "data.choose_a_session_file") }}
        <input type="file" accept="application/json,.json" hidden :disabled="importing || !workspace" :aria-label="t('data.choose_a_session_json_file')" @change="take" />
      </label>
    </div>
  </section>

  <section class="grp">
    <h3 class="lbl">{{ t("data.recover_older_sessions") }}</h3>
    <div class="item">
      <div class="l">
        <b>{{ t("data.import_from_an_older_reasonix") }}</b>
        <span class="hint">{{ t("data.choose_the_older_data_folder") }}</span>
        <span v-if="recovery" class="ready">
          {{ recovery.imported > 0
            ? t("data.recovered_n_sessions", { n: recovery.imported })
            : recovery.recognised ? t("data.nothing_new_to_import") : t("data.no_older_sessions_were_found") }}
          <template v-if="recovery.warnings > 0"> {{ t("data.n_items_could_not_be", { n: recovery.warnings }) }}</template>
        </span>
        <span v-if="error" class="held" role="alert">{{ error }}</span>
      </div>
      <button type="button" class="btn" data-action="session.import-legacy" :disabled="scanning || !workspace" @click="pick">
        {{ t(scanning ? "data.scanning" : "data.choose_older_data_folder") }}
      </button>
    </div>
    <form v-if="typing" class="item" data-action="session.import-legacy" data-value="path" @submit="submit">
      <div class="l">
        <span class="hint">{{ t("data.the_folder_panel_could_not") }}</span>
        <input
          :value="typed"
          :spellcheck="false"
          dir="ltr"
          placeholder="/home/you/.reasonix"
          :aria-label="t('data.older_data_folder_path')"
          data-action="session.import-legacy"
          data-value="path"
          @input="typed = ($event.target as HTMLInputElement).value"
        />
      </div>
      <button type="submit" class="btn" data-action="session.import-legacy" data-value="path" :disabled="scanning || !typed.trim()">
        {{ t("data.import_this_folder") }}
      </button>
    </form>
    <ul v-if="recovery && recovery.skipped.length > 0" class="refusals">
      <li v-for="skip in recovery.skipped" :key="skip.path">{{ skip.name }} — {{ t(SKIP_LABEL[skip.reason] ?? "data.this_folder_could_not_be") }} ({{ skip.path }})</li>
    </ul>
  </section>
</template>
