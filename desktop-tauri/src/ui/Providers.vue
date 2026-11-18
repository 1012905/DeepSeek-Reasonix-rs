<script lang="ts">
import { computed, nextTick, onMounted, ref, watch, type ComponentPublicInstance } from "vue";
import Clip from "./Clip.vue";
import { useEscape } from "./dismiss";
import { t } from "../i18n";
import type { Protocol, ProviderCheck, ProviderCheckDraft, ProviderDraft, ProviderEdit, ProviderEntry, ProviderModelCheck, ProviderModelCheckRequest, ProviderProbe, ProviderPreset, ProviderPresetInstall, ProviderRemoval } from "../port/port";
import AddProvider from "./AddProvider.vue";
import ProviderDetail from "./ProviderDetail.vue";
import { accountKey, accountLabel, disambiguate, hostOf } from "./vendors";
import { reason } from "../i18n/kernel";
import { providerDot } from "./providerdot";
import ServiceMark from "./ServiceMark.vue";
import { moveAccount, orderAccounts, useProviderOrder, writeProviderOrder } from "../state/providerorder";
import StudioIcon from "./StudioIcon.vue";
import RemovalNotice, { removalChangedRoles, type Removal } from "./ProviderRemoval.vue";

// A connection is an account, not a config row. One endpoint answering two
// protocols is two rows in the file and one service to the person paying for it,
// so the rows group by host and the protocol becomes a switch on the account.

export type Port = {
  providers(): Promise<ProviderEntry[]>;
  protocols(): Promise<Protocol[]>;
  providerPresets(): Promise<ProviderPreset[]>;
  installProviderPreset(req: ProviderPresetInstall): Promise<{ names: string[] }>;
  probeProvider(baseUrl: string, apiKey: string): Promise<ProviderProbe>;
  saveProvider(draft: ProviderDraft): Promise<void>;
  removeProvider(name: string): Promise<ProviderRemoval>;
  checkProvider(name: string, draft?: ProviderCheckDraft): Promise<ProviderCheck>;
  checkProviderModel(request: ProviderModelCheckRequest): Promise<ProviderModelCheck>;
  editProvider(edit: ProviderEdit): Promise<void>;
  renameProvider(names: string[], displayName: string): Promise<void>;
  openExternal(url: string): Promise<void>;
};

// One account: every configured entry that answers on the same host.
export interface Account {
  key: string;
  label: string;
  host: string;
  // The config entry's own name, shown only when one host holds two accounts.
  hint: string;
  // What the account is called when nobody has renamed it.
  derived: string;
  byKind: Record<string, ProviderEntry>;
  kinds: string[];
}

export function groupAccounts(list: ProviderEntry[]): Account[] {
  const out = new Map<string, Account>();
  for (const p of list) {
    const host = hostOf(p.baseUrl);
    const kind = p.kind || "openai";
    let key = accountKey(host, p.keyEnv);
    // A second entry for a door the account already has is a duplicate, and a
    // duplicate is still somebody's config: it gets its own row, never a seat
    // behind the first one.
    if (out.get(key)?.byKind[kind]) key = `${key}\u0000${p.name}`;
    let a = out.get(key);
    if (!a) {
      a = { key, label: "", host, hint: p.name, derived: "", byKind: {}, kinds: [] };
      out.set(key, a);
    }
    a.byKind[kind] = p;
    a.kinds.push(kind);
  }
  // Every door has to be in hand before the account can be named: the one the
  // user renamed is not always the first the config file lists.
  for (const a of out.values()) {
    const entries = Object.values(a.byKind);
    a.label = accountLabel(a.host, entries);
    a.derived = accountLabel(a.host, entries.map((e) => ({ ...e, displayName: undefined })));
  }
  return disambiguate([...out.values()]);
}

const SEARCH_FROM = 6;
// The kernel counts characters; maxLength counts UTF-16 units, so this can only
// stop short of the kernel's limit, never past it.
const DISPLAY_NAME_MAX = 64;

const shownName = (a: Account) => Object.values(a.byKind).find((e) => e.displayName?.trim())?.displayName?.trim() ?? "";
</script>

<script setup lang="ts">
const props = defineProps<{
  port: Port;
  onChanged: () => void;
  // A switch that was refused has to say so; silence reads as a click that
  // missed, and the page above already has one place to say it.
  onFailed: (why: string) => void;
  // Which protocol each account is showing, and how to change it. The model
  // list reads the same map, so switching here re-lists the models there.
  protocol: Record<string, string>;
  onProtocol: (account: Account, kind: string) => void;
  activeKindFor: (account: Account) => string;
  // The source whose editor opens on its reasoning fields, when the composer
  // sent the user here to declare effort levels.
  declare?: string;
}>();

const list = ref<ProviderEntry[] | null>(null);
const adding = ref(false);
useEscape(adding, () => { adding.value = false; });
const busy = ref("");
const removal = ref<Removal | null>(null);
const picked = ref("");
const q = ref("");
const renaming = ref("");
const dirty = ref(false);
const asked = ref<{ go: () => void; from: HTMLElement | null } | null>(null);
const pmain = ref<HTMLDivElement | null>(null);
const keepBtn = ref<HTMLButtonElement | null>(null);
const renameText = ref("");
const renameBox = ref<HTMLInputElement | null>(null);
const order = useProviderOrder();
const rows = new Map<string, HTMLButtonElement>();
// The accounts that existed when an add began: the one that is new afterwards
// is the one just added, and it is what the detail should show.
const before = ref<Set<string> | null>(null);

const reload = () => props.port.providers().then((items) => { list.value = items; }).catch(() => { list.value = []; });
onMounted(() => { void reload(); });
watch(() => props.port, () => { void reload(); });

const accounts = computed(() => (list.value ? orderAccounts(groupAccounts(list.value), order.value) : []));
const takenNames = computed(() => (list.value ?? []).map((p) => p.name));
const known = computed(() => list.value ?? []);

function setPicked(key: string) {
  removal.value = null;
  picked.value = key;
}

watch(list, (items) => {
  if (!items || !before.value) return;
  const previous = before.value;
  before.value = null;
  const fresh = groupAccounts(items).find((a) => !previous.has(a.key));
  if (fresh) setPicked(fresh.key);
});

// Until someone picks, the page shows the service it was sent to, else the
// one in use, else the first.
const selected = computed(() => {
  const items = accounts.value;
  if (items.some((a) => a.key === picked.value)) return picked.value;
  const declared = props.declare ? items.find((a) => Object.values(a.byKind).some((e) => e.name === props.declare)) : undefined;
  const active = items.find((a) => Object.values(a.byKind).some((e) => e.inUse));
  return (declared ?? active ?? items[0])?.key ?? "";
});
const current = computed(() => accounts.value.find((a) => a.key === selected.value));
const currentKind = computed(() => {
  const account = current.value;
  return account ? props.protocol[account.key] ?? props.activeKindFor(account) : "";
});
const query = computed(() => q.value.trim().toLowerCase());
const shown = computed(() => accounts.value.filter((a) => !query.value || a.label.toLowerCase().includes(query.value) || a.host.toLowerCase().includes(query.value)));
const rowsView = computed(() => shown.value.map((account, index) => ({ a: account, index, dot: providerDot(Object.values(account.byKind)) })));

const setRow = (key: string, el: Element | ComponentPublicInstance | null) => {
  if (el instanceof HTMLButtonElement) rows.set(key, el);
  else rows.delete(key);
};

const setRenameInput = (el: Element | ComponentPublicInstance | null) => {
  renameBox.value = el instanceof HTMLInputElement ? el : null;
};
watch(renaming, (key) => {
  if (!key) return;
  void nextTick(() => {
    renameBox.value?.focus();
    renameBox.value?.select();
  });
});

const remove = async (name: string) => {
  busy.value = name;
  props.onFailed("");
  removal.value = null;
  const key = selected.value;
  try {
    const report = await props.port.removeProvider(name);
    if (removalChangedRoles(report)) removal.value = { key, name, why: "", report };
  } catch (e) {
    removal.value = { key, name, why: reason(e), report: null };
  } finally {
    void reload();
    props.onChanged();
    busy.value = "";
  }
};

// An empty name, or the one it would be called anyway, clears the label so
// the account follows its derived name again.
const rename = async (a: Account, typed: string) => {
  renaming.value = "";
  const value = typed.trim();
  const next = value === a.derived ? "" : value;
  if (next === shownName(a)) return;
  const names = Object.values(a.byKind).map((e) => e.name);
  busy.value = `rename:${a.key}`;
  props.onFailed("");
  try {
    await props.port.renameProvider(names, next);
    void reload();
    props.onChanged();
  } catch (e) {
    props.onFailed(reason(e));
  } finally {
    busy.value = "";
  }
};

// A form with pending edits is replaced only after the person says so. A save
// in flight is not interrupted: it lands on its own and the form is clean after.
const leave = (go: () => void, leaving = true) => {
  if (!leaving || !dirty.value) go();
  else if (!busy.value.startsWith("edit:")) {
    const from = document.activeElement;
    asked.value = { go, from: from instanceof HTMLElement && from !== document.body ? from : null };
  }
};
watch(dirty, (value) => { if (!value) asked.value = null; });
watch(asked, (value) => { if (value) void nextTick(() => keepBtn.value?.focus()); });

// Focus goes back to the control that asked to leave, or to the form's first
// field when that control is gone.
const keepEditing = () => {
  const back = asked.value?.from?.isConnected ? asked.value.from : pmain.value?.querySelector<HTMLElement>("input:not(:disabled)");
  asked.value = null;
  back?.focus();
};

const discard = () => {
  const go = asked.value?.go;
  asked.value = null;
  dirty.value = false;
  go?.();
};

const leaveKey = (e: KeyboardEvent) => {
  if (e.key !== "Escape") return;
  e.stopPropagation();
  keepEditing();
};

const openAccount = (key: string) => {
  adding.value = false;
  setPicked(key);
};

const startRename = (key: string) => leave(() => {
  adding.value = false;
  setPicked(key);
  const account = accounts.value.find((a) => a.key === key);
  renameText.value = account ? shownName(account) || account.derived : "";
  renaming.value = key;
}, key !== selected.value);

const move = (key: string, direction: -1 | 1) => {
  const next = moveAccount(order.value, accounts.value.map((a) => a.key), key, direction);
  if (!next) return;
  if (!picked.value) setPicked(selected.value);
  rows.get(key)?.focus();
  if (writeProviderOrder(next)) props.onFailed("");
  else props.onFailed(t("models.could_not_save_the_service"));
};

const rowKey = (e: KeyboardEvent, a: Account) => {
  if (e.key === "F2") {
    e.preventDefault();
    startRename(a.key);
    return;
  }
  if (!query.value && e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey
    && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
    e.preventDefault();
    move(a.key, e.key === "ArrowUp" ? -1 : 1);
  }
};

const renameKey = (e: KeyboardEvent, a: Account) => {
  if (e.key === "Enter") {
    e.preventDefault();
    (e.currentTarget as HTMLInputElement).blur();
  } else if (e.key === "Escape") {
    // Abandoning a rename is not closing the settings page.
    e.stopPropagation();
    renameText.value = shownName(a) || a.derived;
    (e.currentTarget as HTMLInputElement).blur();
  }
};

const onAdded = () => {
  before.value = new Set(accounts.value.map((a) => a.key));
  adding.value = false;
  void reload();
  props.onChanged();
};

const onEdited = () => {
  const fresh = reload();
  props.onChanged();
  return fresh;
};

const setBusy = (value: string) => { busy.value = value; };
const setDirty = (value: boolean) => { dirty.value = value; };
const reloadQuiet = () => { void reload(); };

const pickProtocol = (kind: string) => {
  const account = current.value;
  if (account) leave(() => props.onProtocol(account, kind));
};
const renameCurrent = () => {
  const account = current.value;
  if (account) startRename(account.key);
};
const currentRemoveFailed = computed(() => {
  const report = removal.value;
  const account = current.value;
  return report && account && report.key === account.key ? report.why : "";
});
// A removal's outcome shows once the service it concerns has no detail to
// carry it: the roles it handed on, or a failure after it was already written.
const removalNotices = computed<Removal[]>(() => {
  const report = removal.value;
  if (!report) return [];
  return report.report || !accounts.value.some((a) => a.key === report.key) ? [report] : [];
});
</script>

<template>
  <p v-if="list === null" class="acct-note">{{ t("common.loading") }}</p>
  <div v-else class="psplit">
    <div class="plist">
      <input v-if="accounts.length >= SEARCH_FROM" class="psearch" type="search" :value="q" spellcheck="false"
        :placeholder="t('models.search_added_services')" :aria-label="t('models.search_added_services')"
        @input="q = ($event.target as HTMLInputElement).value" />
      <div class="plist-rows" role="group" :aria-label="t('models.n_sources', { n: accounts.length })">
        <template v-for="row in rowsView" :key="row.a.key">
          <div v-if="renaming === row.a.key" class="svcrow-wrap" data-selected="true" data-renaming="">
            <input class="svcrow-name" :ref="setRenameInput" :maxlength="DISPLAY_NAME_MAX" spellcheck="false"
              :value="renameText"
              :placeholder="row.a.derived"
              :aria-label="t('models.rename_name', { name: row.a.label })"
              :title="t('models.enter_saves_esc_cancels_leave', { name: row.a.derived })"
              data-action-keydown="provider.rename" :data-target="row.a.key"
              @focus="($event.target as HTMLInputElement).select()"
              @input="renameText = ($event.target as HTMLInputElement).value"
              @blur="rename(row.a, renameText)"
              @keydown="renameKey($event, row.a)" />
          </div>
          <div v-else class="svcrow-wrap" :data-selected="!adding && row.a.key === selected">
            <button class="svcrow" data-action-click="provider.select" data-action-keydown="provider.move" :data-target="row.a.key"
              :ref="(el: Element | ComponentPublicInstance | null) => setRow(row.a.key, el)"
              :aria-pressed="!adding && row.a.key === selected"
              @keydown="rowKey($event, row.a)"
              @click="leave(() => openAccount(row.a.key), row.a.key !== selected)">
              <ServiceMark :mark="Object.values(row.a.byKind).find((e) => e.mark)?.mark" :id="row.a.key" />
              <span class="tx">
                <span class="nm">{{ row.a.label }}</span>
                <Clip class="ds" :children="row.a.host" />
              </span>
              <i class="pstate" :data-state="row.dot.state" :title="row.dot.title" />
            </button>
            <span class="svcrow-order">
              <button data-action="provider.rename-start" :data-target="row.a.key" :disabled="busy !== ''"
                :aria-label="t('models.rename_name_f2', { name: row.a.label })"
                :title="t('models.rename_name_f2', { name: row.a.label })"
                @click="startRename(row.a.key)"><StudioIcon name="edit" /></button>
              <button v-if="row.index > 0" data-action="provider.move" :data-target="row.a.key" data-value="up"
                :aria-label="t('models.move_name_up_alt_up', { name: row.a.label })"
                :title="t('models.move_name_up_alt_up', { name: row.a.label })"
                @click="move(row.a.key, -1)"><StudioIcon name="arrow" /></button>
              <button v-if="row.index < shown.length - 1" data-action="provider.move" :data-target="row.a.key" data-value="down"
                :aria-label="t('models.move_name_down_alt_down', { name: row.a.label })"
                :title="t('models.move_name_down_alt_down', { name: row.a.label })"
                @click="move(row.a.key, 1)"><StudioIcon name="arrow" class="svcrow-arrow-down" /></button>
            </span>
          </div>
        </template>
        <div v-if="accounts.length === 0" class="empty">{{ t("models.no_model_sources_configured_yet") }}</div>
        <div v-if="accounts.length > 0 && shown.length === 0" class="empty">{{ t("models.no_matching_service") }}</div>
      </div>
      <button class="act padd" data-action="provider.add-start" :aria-pressed="adding" @click="leave(() => (adding = true))">
        <b aria-hidden="true">＋</b>{{ t("models.add_model_service") }}
      </button>
    </div>
    <div ref="pmain" class="pmain">
      <RemovalNotice v-for="notice in removalNotices" :key="notice.key" :removal="notice" />
      <div v-if="asked" class="wsconfirm" role="alertdialog" aria-labelledby="provider-leave-q" aria-describedby="provider-leave-q"
        data-action-keydown="layer.dismiss" @keydown="leaveKey">
        <div class="wsconfirm-t">
          <span id="provider-leave-q" class="q">{{ t("models.this_service_has_unsaved_changes") }}</span>
          <span class="h">{{ t("common.they_will_be_lost_if") }}</span>
        </div>
        <div class="wsconfirm-a">
          <button ref="keepBtn" data-primary data-action="layer.dismiss" @click="keepEditing">{{ t("models.keep_editing") }}</button>
          <button data-action="provider.discard-edits" data-danger @click="discard">
            {{ t("models.discard_changes_and_leave") }}
          </button>
        </div>
      </div>
      <AddProvider v-if="adding" :port="port" :taken="takenNames" :known="known" :on-done="onAdded" :on-cancel="() => (adding = false)" />
      <ProviderDetail v-else-if="current" :key="current.key" :a="current" :port="port" :busy="busy" :set-busy="setBusy"
        :kind="currentKind" :on-protocol="pickProtocol" :on-remove="remove"
        :remove-failed="currentRemoveFailed" :on-rename="renameCurrent" :declare="declare"
        :on-edited="onEdited" :on-checked="reloadQuiet" :on-dirty="setDirty" />
      <div v-else class="empty">{{ t("models.add_a_model_service_to") }}</div>
    </div>
  </div>
</template>
