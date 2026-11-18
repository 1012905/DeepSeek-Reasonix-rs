<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { BackupApplyResult, BackupPlan, BackupPlanItem } from "../../../port/backup";

// A restore is a preview then an apply, and both are the kernel's: which rows
// need consent, and why, is decided there and rechecked when it is applied. This
// panel only collects two separate answers per row — take it, and where asked,
// allow what it does.

const STATUS_LABEL: Record<BackupPlanItem["status"], string> = {
  new: "data.not_on_this_machine",
  changed: "data.differs_from_this_machine",
  same: "data.same_as_this_machine",
  install: "data.needs_install",
};

const CONSENT_LABEL: Record<string, string> = {
  executes: "data.i_have_read_this_command",
  endpoint: "data.i_allow_conversations_and_the",
  replaces_secret: "data.i_allow_the_key_in",
  imports: "data.i_have_read_this_instruction",
};

const props = defineProps<{
  name: string;
  preview: (passphrase: string) => Promise<BackupPlan>;
  apply: (planId: string, items: string[], consented: string[]) => Promise<BackupApplyResult>;
  // Which rows start ticked. A file from elsewhere starts by adding what this
  // machine lacks; the kernel's own recommendation is the default.
  preselect?: (item: BackupPlanItem) => boolean;
}>();

const pass = ref("");
const plan = ref<BackupPlan | null>(null);
const picked = ref<ReadonlySet<string>>(new Set<string>());
const allowed = ref<ReadonlySet<string>>(new Set<string>());
const result = ref<BackupApplyResult | null>(null);
const busy = ref(false);
const error = ref("");

const flip = (set: ReadonlySet<string>, id: string, on: boolean) => {
  const next = new Set(set);
  if (on) next.add(id);
  else next.delete(id);
  return next as ReadonlySet<string>;
};

async function open(): Promise<void> {
  busy.value = true;
  error.value = "";
  try {
    const p = await props.preview(pass.value);
    plan.value = p;
    picked.value = new Set(p.items.filter((i) => (props.preselect ?? ((x: BackupPlanItem) => x.recommended))(i)).map((i) => i.id));
    allowed.value = new Set<string>();
    pass.value = "";
    result.value = null;
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// A row that asks permission and has not been given it blocks the apply, so the
// button says what is missing instead of failing at the kernel.
const blocked = computed(() => (plan.value?.items ?? []).filter((i) => picked.value.has(i.id) && i.consent && !allowed.value.has(i.id)).length);
const takes = computed(() => (plan.value?.items ?? []).filter((i) => picked.value.has(i.id)).length);

async function commit(): Promise<void> {
  const held = plan.value;
  if (!held || blocked.value > 0) return;
  busy.value = true;
  error.value = "";
  try {
    const ids = held.items.filter((i) => picked.value.has(i.id)).map((i) => i.id);
    result.value = await props.apply(held.planId, ids, ids.filter((id) => allowed.value.has(id)));
    plan.value = null;
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

const close = () => { plan.value = null; error.value = ""; pass.value = ""; };

const statusSaid = (item: BackupPlanItem): string => t(STATUS_LABEL[item.status] ?? item.status);
</script>

<template>
  <div class="bk-restore">
    <p v-if="!plan" class="acct-note">{{ t("data.enter_the_passphrase_for_name", { name }) }}</p>
    <div v-if="!plan" class="bk-fields">
      <label>
        <span>{{ t("data.passphrase") }}</span>
        <input data-action="backup.passphrase" data-value="restore" type="password" autoComplete="off" :value="pass" :disabled="busy" @input="pass = ($event.target as HTMLInputElement).value" />
      </label>
    </div>
    <div v-if="!plan" class="bk-acts">
      <button type="button" class="btn" data-action="backup.preview" data-primary :disabled="busy || !pass" @click="open">
        {{ t(busy ? "data.decrypting" : "data.preview_differences") }}
      </button>
    </div>

    <template v-if="plan">
      <p class="acct-note">{{ plan.platform }} · {{ plan.samePlatform ? t("data.same_as_this_machine") : t("data.from_another_system_it_may") }}</p>
      <div class="bk-plan">
        <div v-for="item in plan.items" :key="item.id" class="bk-item" :data-status="item.status">
          <label class="take">
            <input type="checkbox" data-action="backup.pick" :data-target="item.id" :checked="picked.has(item.id)" :disabled="busy" @change="picked = flip(picked, item.id, ($event.target as HTMLInputElement).checked)" />
            <span class="nm">{{ item.name }}</span>
            <span class="cat">{{ item.kind || item.category }}</span>
            <span class="st">{{ statusSaid(item) }}</span>
          </label>
          <p v-if="item.summary" class="sum">{{ item.summary }}</p>
          <p v-if="item.crossPlatform" class="warn">{{ t("data.commands_and_paths_may_not") }}</p>
          <label v-if="item.consent" class="allow">
            <input type="checkbox" data-action="backup.allow" :data-target="item.id" :checked="allowed.has(item.id)" :disabled="busy" @change="allowed = flip(allowed, item.id, ($event.target as HTMLInputElement).checked)" />
            <span>{{ t(CONSENT_LABEL[item.consent] ?? item.consent) }}</span>
          </label>
        </div>
      </div>
      <div class="bk-acts">
        <button type="button" class="btn" data-action="backup.apply" data-primary :disabled="busy || takes === 0 || blocked > 0" @click="commit">
          {{ t(busy ? "data.restoring" : "data.restore_n_selected", { n: takes }) }}
        </button>
        <button type="button" class="btn" data-action="backup.cancel" :disabled="busy" @click="close">{{ t("common.cancel") }}</button>
        <span v-if="blocked > 0" class="acct-note">{{ t("data.n_items_still_need_confirmation", { n: blocked }) }}</span>
      </div>
    </template>

    <div v-if="result" class="bk-restore" role="status">
      <p class="acct-note">{{ t("data.restored_n_items", { n: result.applied.length }) }}</p>
      <p v-if="result.failed?.length" class="acct-note" data-err="">{{ result.failed.map((f) => `${f.id}: ${f.error}`).join("；") }}</p>
      <p v-if="result.reloadError" class="acct-note" data-err="">{{ result.reloadError }}</p>
    </div>
    <p v-if="error" class="acct-note" data-err="" role="alert">{{ error }}</p>
  </div>
</template>
