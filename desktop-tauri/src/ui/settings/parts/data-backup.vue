<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { bytes } from "../../../i18n/format";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import { CATEGORY_LABEL, CATEGORY_NOTE } from "./data-backup-categories";
import type { BackupCatalog, BackupCategory, BackupEntry } from "../../../port/backup";
import BackupRestore from "./data-backup-restore.vue";

// A backup lives in the signed-in account, so this block is drawn only once one
// exists. The kernel seals every archive with the person's passphrase before it
// leaves; the service stores the envelope and the metadata listed here.
const props = defineProps<{ port: AgentPort }>();

const catalog = ref<BackupCatalog | null>(null);
const error = ref("");
const chosen = ref<ReadonlySet<BackupCategory> | null>(null);
const label = ref("");
const pass = ref("");
const again = ref("");
const busy = ref(false);
const note = ref("");
const confirm = ref("");
const restoring = ref<BackupEntry | null>(null);

async function load(): Promise<void> {
  try {
    const c = await props.port.backups();
    catalog.value = c;
    chosen.value ??= new Set(c.categories.filter((x) => x.defaultOn).map((x) => x.id));
    error.value = "";
  } catch (e) {
    error.value = reason(e);
  }
}

watch(() => props.port, () => { void load(); }, { immediate: true });

const flip = (id: BackupCategory) => {
  const next = new Set(chosen.value ?? []);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  chosen.value = next;
};

const min = computed(() => catalog.value?.minPassphrase ?? 10);
const mismatch = computed(() => again.value !== "" && pass.value !== again.value);
const count = computed(() => chosen.value?.size ?? 0);
const ready = computed(() => count.value > 0 && pass.value.length >= min.value && pass.value === again.value && !busy.value);
const wait = computed(() =>
  count.value === 0
    ? t("data.tick_at_least_one_thing")
    : pass.value.length < min.value
      ? t("data.passphrase_too_short_n_more", { n: min.value - pass.value.length })
      : again.value === ""
        ? t("data.repeat_the_passphrase_to_confirm")
        : "");

async function create(): Promise<void> {
  const held = catalog.value;
  if (!held) return;
  busy.value = true;
  note.value = "";
  error.value = "";
  try {
    const order = held.categories.map((c) => c.id).filter((id) => chosen.value?.has(id));
    const out = await props.port.createBackup({ label: label.value.trim(), categories: order, passphrase: pass.value });
    pass.value = "";
    again.value = "";
    label.value = "";
    const skipped = out.omitted?.length ?? 0;
    note.value = skipped ? t("data.backed_up_n_files_were", { n: skipped }) : t("data.backed_up");
    await load();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// Removing one is asked twice: the second press is the confirmation, and the
// first is what keeps a stray click from spending a backup nobody can restore.
async function drop(id: string): Promise<void> {
  if (confirm.value !== id) { confirm.value = id; return; }
  confirm.value = "";
  busy.value = true;
  error.value = "";
  try {
    await props.port.deleteBackup(id);
    await load();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// Translated once here rather than at each row: the label map is the same for
// every backup, and a call in the template made the row read as prose.
const labels = Object.fromEntries(Object.entries(CATEGORY_LABEL).map(([id, key]) => [id, t(key)])) as Record<string, string>;

const at = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
};

const preview = (id: string) => (passphrase: string) => props.port.previewBackup(id, passphrase);
const apply = (planId: string, items: string[], consented: string[]) => props.port.applyBackup(planId, items, consented);
</script>

<template>
  <p v-if="error && !catalog" class="acct-note" data-err="" role="alert">{{ error }}</p>
  <p v-else-if="!catalog" class="acct-note">{{ t("data.loading_backups") }}</p>
  <div v-else class="bk">
    <p class="acct-note">{{ t("data.a_backup_is_encrypted_here") }}</p>
    <div class="bk-cats" role="group" :aria-label="t('data.what_to_back_up')">
      <label v-for="c in catalog.categories" :key="c.id" class="bk-cat" :data-warn="c.id === 'secrets' && chosen?.has(c.id) ? '' : undefined">
        <input type="checkbox" data-action="backup.category" :data-target="c.id" :checked="!!chosen?.has(c.id)" :disabled="busy" @change="flip(c.id)" />
        <span class="nm">{{ t(CATEGORY_LABEL[c.id] ?? c.id) }}</span>
        <span class="why">{{ t(CATEGORY_NOTE[c.id] ?? c.id) }}</span>
      </label>
    </div>

    <div class="bk-fields">
      <label class="full">
        <span>{{ t("data.label_optional") }}</span>
        <input data-action="backup.label" :value="label" :disabled="busy" maxlength="80" :placeholder="t('data.e_g_work_laptop')" @input="label = ($event.target as HTMLInputElement).value" />
      </label>
      <label>
        <span>{{ t("data.passphrase_at_least_n_characters", { n: min }) }}</span>
        <input data-action="backup.passphrase" data-target="new" type="password" autoComplete="new-password" :value="pass" :disabled="busy" @input="pass = ($event.target as HTMLInputElement).value" />
      </label>
      <label>
        <span>{{ t("data.repeat_it") }}</span>
        <input data-action="backup.passphrase" data-target="again" type="password" autoComplete="new-password" :value="again" :disabled="busy" @input="again = ($event.target as HTMLInputElement).value" />
      </label>
    </div>

    <p v-if="mismatch" class="acct-note" data-err="">{{ t("data.the_two_passphrases_differ") }}</p>
    <div class="bk-acts">
      <button type="button" class="btn" data-primary data-action="backup.create" :disabled="!ready" @click="create">
        {{ t(busy ? "data.encrypting_and_uploading" : "data.back_up_to_account") }}
      </button>
      <span v-if="!ready && !busy" class="acct-note bk-wait">{{ wait }}</span>
    </div>
    <p v-if="note" class="acct-note">{{ note }}</p>
    <p v-if="error" class="acct-note" data-err="" role="alert">{{ error }}</p>

    <p v-if="catalog.backups.length === 0" class="acct-note">{{ t("data.no_backups_in_this_account") }}</p>
    <ul v-else class="bk-list">
      <li v-for="b in catalog.backups" :key="b.id" class="bk-row">
        <span class="who">
          <span class="nm">{{ b.label || t("data.untitled_backup") }}</span>
          <span class="meta">{{ `${at(b.createdAt)} · ${b.appVersion} · ${b.platform} · ${bytes(b.ciphertextBytes)}` }}</span>
          <span class="meta">{{ b.categories.map((c) => labels[c] ?? c).join("、") }}</span>
        </span>
        <button type="button" class="btn sm" data-action="backup.restore" :data-target="b.id" :disabled="busy" @click="restoring = b">
          {{ t("data.restore") }}
        </button>
        <button type="button" class="btn sm" data-action="backup.delete" :data-target="b.id" :disabled="busy" @click="drop(b.id)">
          {{ t(confirm === b.id ? "common.confirm_delete" : "common.delete") }}
        </button>
      </li>
    </ul>
    <BackupRestore
      v-if="restoring"
      :name="restoring.label || t('data.untitled_backup')"
      :preview="preview(restoring.id)"
      :apply="apply"
    />
  </div>
</template>
