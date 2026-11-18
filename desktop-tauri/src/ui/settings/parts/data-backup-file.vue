<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import { CATEGORY_LABEL, CATEGORY_NOTE } from "./data-backup-categories";
import type { BackupCategory, BackupFileCatalog } from "../../../port/backup";
import StudioIcon from "../../StudioIcon.vue";
import BackupRestore from "./data-backup-restore.vue";

// The same sealed envelope the account keeps, written to a file the person
// chooses. It asks nothing of the account: the passphrase is the only secret,
// and the kernel offers this route to the window alone.

const props = defineProps<{ port: AgentPort }>();

const catalog = ref<BackupFileCatalog | null>(null);
const unavailable = ref(false);
const chosen = ref<ReadonlySet<BackupCategory>>(new Set<BackupCategory>());
const pass = ref("");
const again = ref("");
const busy = ref(false);
const note = ref("");
const error = ref("");
const restoring = ref(false);
const text = ref("");

watch(() => props.port, (port) => {
  let live = true;
  port.backupFileCatalog()
    .then((c) => {
      if (!live) return;
      catalog.value = c;
      chosen.value = new Set(c.categories.filter((x) => x.defaultOn).map((x) => x.id));
    })
    .catch(() => { if (live) unavailable.value = true; });
  return () => { live = false; };
}, { immediate: true });

const flip = (id: BackupCategory) => {
  const next = new Set(chosen.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  chosen.value = next;
};

const min = () => catalog.value?.minPassphrase ?? 10;
const mismatch = () => again.value !== "" && pass.value !== again.value;
const ready = () =>
  chosen.value.size > 0 && pass.value.length >= min() && pass.value === again.value && !busy.value;
// The button says what is still missing rather than sitting greyed with no reason.
const wait = () =>
  chosen.value.size === 0
    ? t("data.tick_at_least_one_thing")
    : pass.value.length < min()
      ? t("data.passphrase_too_short_n_more", { n: min() - pass.value.length })
      : again.value === ""
        ? t("data.repeat_the_passphrase_to_confirm")
        : "";

async function exportFile(): Promise<void> {
  const held = catalog.value;
  if (!held) return;
  busy.value = true;
  note.value = "";
  error.value = "";
  try {
    const order = held.categories.map((c) => c.id).filter((id) => chosen.value.has(id));
    const out = await props.port.exportBackupFile(order, pass.value);
    await props.port.saveText(out.name, out.file);
    pass.value = "";
    again.value = "";
    const skipped = out.omitted?.length ?? 0;
    note.value = skipped ? t("data.exported_n_files_were_too", { n: skipped }) : t("data.exported");
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

async function choose(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  error.value = "";
  try {
    text.value = (await file.text()).trim();
    restoring.value = true;
  } catch (e) {
    error.value = reason(e);
  }
}

const preview = (passphrase: string) => props.port.previewBackupFile(text.value, passphrase);
const apply = (planId: string, items: string[], consented: string[]) => props.port.applyBackupFile(planId, items, consented);
</script>

<template>
  <p v-if="unavailable" class="acct-note">{{ t("data.exporting_and_importing_a_config") }}</p>
  <p v-else-if="!catalog" class="acct-note">{{ t("common.loading") }}</p>
  <div v-else class="bk">
    <p class="acct-note">{{ t("data.the_file_is_encrypted_on") }}</p>
    <div class="bk-cats bk-chips" role="group" :aria-label="t('data.what_to_export')">
      <label
        v-for="c in catalog.categories"
        :key="c.id"
        class="bk-chip"
        :data-on="chosen.has(c.id) ? '' : undefined"
        :data-warn="c.id === 'secrets' && chosen.has(c.id) ? '' : undefined"
        :title="t(c.id === 'secrets' ? 'data.off_by_default_if_ticked' : CATEGORY_NOTE[c.id])"
      >
        <input type="checkbox" data-action="backup.category" :data-target="c.id" :checked="chosen.has(c.id)" :disabled="busy" @change="flip(c.id)" />
        <span class="box" aria-hidden="true"><StudioIcon v-if="chosen.has(c.id)" name="check" /></span>
        {{ t(CATEGORY_LABEL[c.id] ?? c.id) }}
      </label>
    </div>
    <p v-if="chosen.has('secrets')" class="acct-note bk-wait">{{ t("data.off_by_default_if_ticked") }}</p>
    <div class="bk-fields">
      <label>
        <span>{{ t("data.passphrase_at_least_n_characters", { n: min() }) }}</span>
        <input data-action="backup.passphrase" data-target="new" type="password" autoComplete="new-password" :value="pass" :disabled="busy" @input="pass = ($event.target as HTMLInputElement).value" />
      </label>
      <label>
        <span>{{ t("data.repeat_it") }}</span>
        <input data-action="backup.passphrase" data-target="again" type="password" autoComplete="new-password" :value="again" :disabled="busy" @input="again = ($event.target as HTMLInputElement).value" />
      </label>
    </div>
    <p v-if="mismatch()" class="acct-note" data-err="">{{ t("data.the_two_passphrases_differ") }}</p>
    <div class="bk-acts">
      <button type="button" class="btn" data-primary data-action="backup.file-export" :disabled="!ready()" @click="exportFile">
        {{ t(busy ? "data.encrypting" : "data.export_to_file") }}
      </button>
      <span v-if="!ready() && !busy && wait()" class="acct-note bk-wait">{{ wait() }}</span>
      <label class="btn" data-action="backup.file-import">
        {{ t("data.import_from_file") }}
        <input type="file" accept=".rxcb,text/plain" hidden :disabled="busy" @change="choose" />
      </label>
    </div>
    <p v-if="note" class="acct-note">{{ note }}</p>
    <p v-if="error" class="acct-note" data-err="" role="alert">{{ error }}</p>
    <BackupRestore
      v-if="restoring"
      :name="t('data.import_from_file')"
      :preview="preview"
      :apply="apply"
      :preselect="(item) => item.recommended && item.status === 'new'"
    />
  </div>
</template>
