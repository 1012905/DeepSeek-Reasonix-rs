<script lang="ts">
import type { HubPort, TreeWorkspace } from "../../port/hub";
import type { RemoteHost, RemoteHostEdit } from "../../port/remote";
import { draftOf, workspacesOf } from "./hostbook";

// The machines panel. Declared apart from the component so the window can wire
// it against one declaration.
export interface RemotesProps {
  hub: HubPort;
  /** Opens a pane on a machine. `workspace` is its folder, `sessionPath` an
   *  existing conversation; neither is that machine's default workspace. */
  onOpenRemote: (req: { host: string; workspace?: string; sessionPath?: string }) => void;
  onError: (e: unknown) => void;
  onClose: () => void;
}

/** Re-exported from ./hostbook so a caller of this panel keeps one import: the
 *  settings page's host list reads the same definitions. */
export { draftOf, workspacesOf };

/** The projects a machine holds, as the rail lists them: what the far kernel
 *  answered, then every folder this window wrote down that it did not mention,
 *  then whatever a pane is open on. Nothing is dropped for being unknown to one
 *  of the three, because each of them knows something the others do not. */
export function projectsOf(host: RemoteHost, tree: TreeWorkspace[] | null | undefined): TreeWorkspace[] {
  const holds = (ws: TreeWorkspace, root: string) => ws.root === root || !!ws.workspaceRoots?.includes(root);
  const leaf = (dir: string) => dir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || dir;
  const out: TreeWorkspace[] = tree ? [...tree] : [];
  for (const dir of workspacesOf(host)) {
    if (!out.some((ws) => holds(ws, dir))) out.push({ root: dir, name: leaf(dir), sessions: [] });
  }
  return out;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import type { RemoteProbe, WSLDistros } from "../../port/remote";
import { toggleHiddenHost, useHiddenHosts } from "../../state/remotehide";
import StudioIcon from "../StudioIcon.vue";
import RemoteDirs from "./RemoteDirs.vue";
import RemoteHosts from "./RemoteHosts.vue";

const props = defineProps<RemotesProps>();

const hosts = ref<RemoteHost[] | null>([]);
// Which ssh aliases this machine has, and the distributions on it. Both are
// best-effort: a kernel that cannot answer one leaves the form it has always
// had, which is why neither turns into an error.
const candidates = ref<string[]>([]);
const wsl = ref<WSLDistros | null>(null);
// Null is "no form open". The name a draft is editing stays apart from the
// draft's own, so renaming remains possible without losing the row it came from.
const draft = ref<RemoteHostEdit | null>(null);
const editing = ref("");
// The folder being typed, kept out of the draft: it is not part of the row until
// it is added, and a save must not smuggle a half-typed path in.
const dir = ref("");
const busy = ref("");
const probing = ref("");
const probes = ref<Record<string, RemoteProbe>>({});
// Browsing dials, and only a machine already in the book has an address to dial.
// A draft being typed has nowhere to go yet, so it types the path instead.
const picking = ref(false);
const unavailable = ref(false);

const hidden = useHiddenHosts();
const hiddenSet = computed(() => new Set(hidden.value));

async function load(): Promise<void> {
  try {
    const [book, aliases, distros] = await Promise.all([
      props.hub.remoteHosts(),
      props.hub.remoteCandidates().catch(() => [] as string[]),
      props.hub.wslDistros().catch(() => null),
    ]);
    // Null is a kernel that does not do remote panes at all — a page served to a
    // browser, or a paired device. Nothing here can work, so nothing is drawn.
    if (book === null) {
      unavailable.value = true;
      hosts.value = null;
      return;
    }
    unavailable.value = false;
    hosts.value = book;
    candidates.value = aliases;
    wsl.value = distros?.available ? distros : null;
  } catch (e) {
    props.onError(e);
  }
}

watch(() => props.hub, () => { void load(); }, { immediate: true });

async function save(entry: RemoteHostEdit): Promise<void> {
  busy.value = entry.name;
  try {
    await props.hub.saveRemoteHost(entry);
    draft.value = null;
    editing.value = "";
    dir.value = "";
    await load();
  } catch (e) {
    props.onError(e);
  } finally {
    busy.value = "";
  }
}

async function drop(name: string): Promise<void> {
  busy.value = name;
  try {
    await props.hub.removeRemoteHost(name);
    await load();
  } catch (e) {
    props.onError(e);
  } finally {
    busy.value = "";
  }
}

/** A step is a placement in the book relative to the machines drawn here. The
 *  last one has no next name, and "" is the kernel's "to the end". */
async function move(name: string, direction: -1 | 1): Promise<void> {
  const book = hosts.value ?? [];
  const at = book.findIndex((host) => host.name === name);
  if (at < 0) return;
  if (direction === -1 && at === 0) return;
  if (direction === 1 && at >= book.length - 1) return;
  busy.value = name;
  try {
    await props.hub.placeRemoteHost(name, direction === -1 ? book[at - 1]?.name ?? "" : book[at + 2]?.name ?? "");
    await load();
  } catch (e) {
    props.onError(e);
  } finally {
    busy.value = "";
  }
}

/** A switch is a save of the row as stored with one field flipped: the endpoint
 *  replaces the entry, so a partial copy would blank the rest. */
function toggleOff(host: RemoteHost): void {
  void save({ ...draftOf(host), disabled: !host.disabled });
}

function probe(name: string): void {
  probing.value = name;
  props.hub
    .probeRemote(name)
    .then((report) => { probes.value = { ...probes.value, [name]: report }; })
    .catch(props.onError)
    .finally(() => { probing.value = ""; });
}

// The list is edited whole and saved with the row. Head = default, so promoting
// one is a move rather than a second field that can disagree with it.
function setDirs(next: string[]): void {
  if (draft.value) draft.value = { ...draft.value, workspaces: next };
}

function addDir(): void {
  const path = dir.value.trim();
  if (!path || !draft.value) return;
  if (!(draft.value.workspaces ?? []).includes(path)) setDirs([...(draft.value.workspaces ?? []), path]);
  dir.value = "";
}

function edit(host: RemoteHost): void {
  editing.value = host.name;
  draft.value = draftOf(host);
}

function closeForm(): void {
  draft.value = null;
  editing.value = "";
  dir.value = "";
}

const isWSL = computed(() => draft.value?.kind === "wsl");

// WSL 2 only: the far kernel runs in a distribution's own session, which is
// what version 2 is. A saved distribution stays offered even when this machine
// no longer lists it, or saving a row would silently re-point it.
const distroChoices = computed(() => {
  const distros = (wsl.value?.distros ?? []).filter((distro) => distro.version === 2);
  const saved = draft.value?.distro;
  return saved && !distros.some((distro) => distro.name === saved) ? [{ name: saved, version: 2 }, ...distros] : distros;
});

function chooseKind(kind: string): void {
  if (!draft.value) return;
  const first = (wsl.value?.distros ?? []).find((distro) => distro.version === 2)?.name ?? "";
  draft.value = { ...draft.value, kind, distro: kind ? draft.value.distro || first : "" };
}

function onField(key: keyof RemoteHostEdit, event: Event): void {
  if (!draft.value) return;
  draft.value = { ...draft.value, [key]: (event.target as HTMLInputElement).value };
}

function onPort(event: Event): void {
  if (!draft.value) return;
  draft.value = { ...draft.value, port: Number((event.target as HTMLInputElement).value.replace(/\D/g, "")) || 0 };
}

function onCheck(event: Event): void {
  if (!draft.value) return;
  draft.value = { ...draft.value, useSSHConfig: (event.target as HTMLInputElement).checked };
}

/** Importing beats typing: on a machine that already uses ssh, the addresses are
 *  written down next door and this only borrows the name. */
async function addAll(): Promise<void> {
  for (const alias of candidates.value) await save({ name: alias, useSSHConfig: true });
}

const drawn = computed(() => hosts.value ?? []);
</script>

<template>
  <div class="rmtpanel" role="region" :aria-label="t('settings.other_machines')">
    <header class="rmtpanel-hd">
      <h2>{{ t("settings.other_machines") }}</h2>
      <button type="button" class="rmthost-btn" data-action="remotes.close" :title="t('common.close')" :aria-label="t('common.close')" @click="onClose">
        <StudioIcon name="close" />
      </button>
    </header>

    <!-- A kernel that does not do remote panes says so once, rather than
         offering a form whose saves would all be refused. -->
    <p v-if="unavailable" class="rmtempty" role="alert">{{ t("remote.remote_connection_failed") }}</p>

    <template v-else>
      <div v-if="!draft" class="rmthead">
        <button type="button" class="rmtadd" data-action="remotes.add-ask" @click="draft = draftOf(null)">
          <StudioIcon name="plus" />{{ t("remote.add_a_machine") }}
        </button>
      </div>

      <RemoteHosts
        v-for="(host, position) in drawn"
        :key="host.name"
        :host="host"
        :projects="workspacesOf(host)"
        :position="position"
        :total="drawn.length"
        :probe="probes[host.name]"
        :probing="probing === host.name"
        :busy="busy === host.name"
        :last-used="0"
        :hidden="hiddenSet.has(host.name)"
        :on-probe="() => probe(host.name)"
        :on-open="() => onOpenRemote({ host: host.name, workspace: host.workspace })"
        :on-edit="() => edit(host)"
        :on-remove="() => drop(host.name)"
        :on-toggle-off="() => toggleOff(host)"
        :on-hide="() => toggleHiddenHost(host.name)"
        :on-move="(direction) => move(host.name, direction)"
      />

      <p v-if="drawn.length === 0 && !draft" class="rmtempty">{{ t("remote.no_remote_machines_yet_add") }}</p>

      <div v-if="candidates.length > 0" class="rmtcands">
        <span class="cap">{{ t("remote.also_in_ssh_config") }}</span>
        <button
          v-for="alias in candidates"
          :key="alias"
          type="button"
          class="rmtcand"
          data-action="remotes.add"
          :data-target="alias"
          :disabled="!!busy"
          :title="t('remote.add_it_with_the_settings')"
          @click="save({ name: alias, useSSHConfig: true })"
        >
          <StudioIcon name="plus" /> {{ alias }}
        </button>
        <!-- A sentence rather than an alias, so it is not drawn in the mono
             face the alias chips beside it use. -->
        <button v-if="candidates.length > 1" type="button" class="spill" data-all data-action="remotes.add-all" :disabled="!!busy" @click="addAll">
          {{ t("remote.add_them_all") }}
        </button>
      </div>

      <RemoteDirs
        v-if="picking && editing"
        :hub="hub"
        :host="editing"
        :start="(draft?.workspaces ?? [])[0]"
        :on-pick="(path) => { picking = false; if (!(draft?.workspaces ?? []).includes(path)) setDirs([...(draft?.workspaces ?? []), path]); }"
        :on-close="() => { picking = false; }"
      />

      <form v-if="draft" class="rmtform" @submit.prevent="save(draft)">
        <label class="rmtf">
          <span>{{ t("common.name") }}</span>
          <input :value="draft.name" placeholder="gpu-box" spellcheck="false" dir="ltr" @input="onField('name', $event)" />
        </label>

        <!-- A saved machine keeps the way it is reached: the two kinds share no
             field, so turning one into the other is a new machine, not an edit. -->
        <div v-if="wsl && !editing" class="rmtf" role="radiogroup" :aria-label="t('remote.connection_type')">
          <span>{{ t("remote.connection_type") }}</span>
          <div class="rmtkinds">
            <button
              type="button"
              role="radio"
              data-action="remotes.kind"
              data-value="ssh"
              :aria-checked="(draft.kind ?? '') === ''"
              @click="chooseKind('')"
            >
              {{ t("remote.ssh_host") }}
            </button>
            <button
              type="button"
              role="radio"
              data-action="remotes.kind"
              data-value="wsl"
              :aria-checked="draft.kind === 'wsl'"
              @click="chooseKind('wsl')"
            >
              {{ t("remote.wsl_distribution") }}
            </button>
          </div>
        </div>

        <label v-if="isWSL" class="rmtf">
          <span>{{ t("remote.distribution") }}</span>
          <select data-action-change="remotes.distro" :value="draft.distro ?? ''" :disabled="!!editing" @change="onField('distro', $event)">
            <option v-for="distro in distroChoices" :key="distro.name" :value="distro.name">{{ distro.name }}</option>
          </select>
        </label>

        <template v-if="!isWSL">
          <label class="rmtf">
            <span>{{ t("remote.address") }}</span>
            <input :value="draft.host ?? ''" placeholder="10.0.0.4" spellcheck="false" dir="ltr" @input="onField('host', $event)" />
          </label>
          <label class="rmtf">
            <span>{{ t("remote.user") }}</span>
            <input :value="draft.user ?? ''" placeholder="ada" spellcheck="false" dir="ltr" @input="onField('user', $event)" />
          </label>
          <label class="rmtf">
            <span>{{ t("common.port") }}</span>
            <input :value="draft.port ? String(draft.port) : ''" placeholder="22" inputmode="numeric" dir="ltr" @input="onPort" />
          </label>
          <label class="rmtf">
            <span>{{ t("remote.jump_host") }}</span>
            <input :value="draft.proxyJump ?? ''" placeholder="bastion.example.com" spellcheck="false" dir="ltr" @input="onField('proxyJump', $event)" />
          </label>
        </template>
        <p v-else class="rmthint">{{ t("remote.the_kernel_runs_in_a") }}</p>

        <!-- Same shape the sandbox's writable list uses: one row per folder, then
             what you can do to it. The head carries the default badge rather than
             a separate field, because it is the same folder. -->
        <div class="rmtdirs">
          <div class="sublb">{{ t("remote.projects_on_this_machine") }}</div>
          <div v-for="(path, at) in draft.workspaces ?? []" :key="path" class="prule">
            <code dir="ltr">{{ path }}</code>
            <span v-if="at === 0" class="tag">{{ t("common.default") }}</span>
            <button
              v-else
              type="button"
              class="act ghost"
              data-action="remotes.make-default"
              :data-target="path"
              :aria-label="t('remote.make_path_the_default', { path })"
              @click="setDirs([path, ...(draft.workspaces ?? []).filter((x) => x !== path)])"
            >
              {{ t("common.make_default") }}
            </button>
            <button
              type="button"
              class="act ghost"
              data-action="remotes.unlist-dir"
              :data-target="path"
              :aria-label="t('remote.stop_listing_path', { path })"
              @click="setDirs((draft.workspaces ?? []).filter((x) => x !== path))"
            >
              {{ t("common.delete") }}
            </button>
          </div>
          <div class="prule">
            <input
              v-model="dir"
              :placeholder="t('remote.a_project_directory_over_there')"
              :aria-label="t('remote.a_project_directory_over_there')"
              spellcheck="false"
              dir="ltr"
              @keydown.enter.prevent="addDir"
            />
            <button type="button" class="act" data-action="remotes.list-dir" :disabled="!dir.trim()" @click="addDir">
              {{ t("common.add") }}
            </button>
            <button v-if="editing" type="button" class="act ghost" data-action="remotes.browse" @click="picking = true">
              {{ t("remote.browse") }}
            </button>
          </div>
        </div>

        <template v-if="!isWSL">
          <label class="rmtf">
            <span>{{ t("remote.private_key_file") }}</span>
            <input :value="draft.identityFile ?? ''" placeholder="~/.ssh/id_ed25519" spellcheck="false" dir="ltr" @input="onField('identityFile', $event)" />
          </label>
          <!-- What an empty form does is the kernel's to say. Leaving these blank
               is not "no authentication", and a reader who has to guess between
               agent, default keys and a password prompt finds out on connect. -->
          <p class="rmthint">{{ t("remote.left_blank_it_tries_in") }}</p>
          <!-- Named, not carried: this is the variable to read, never the secret
               itself, so nothing typed here is a password on its way anywhere. -->
          <label class="rmtf">
            <span>{{ t("remote.env_var_holding_the_key") }}</span>
            <input :value="draft.passphraseEnv ?? ''" placeholder="GPU_BOX_PASSPHRASE" spellcheck="false" dir="ltr" @input="onField('passphraseEnv', $event)" />
          </label>
          <label class="rmtf">
            <span>{{ t("remote.env_var_with_the_password") }}</span>
            <input :value="draft.passwordEnv ?? ''" placeholder="GPU_BOX_PASSWORD" spellcheck="false" dir="ltr" @input="onField('passwordEnv', $event)" />
          </label>
        </template>

        <!-- The kernel names this setting when an install fails, so it has to be
             here: an error saying "install it with npm instead" beside a setting
             nobody can reach says nothing. -->
        <label class="rmtf">
          <span>{{ t("remote.installing") }}</span>
          <select :value="draft.serveInstall || 'auto'" :title="t('remote.a_first_connect_installs_a')" @change="onField('serveInstall', $event)">
            <option value="auto">{{ t("common.pick_one_automatically") }}</option>
            <option value="npm">{{ t("remote.use_the_remotes_npm") }}</option>
            <option value="upload">{{ t("remote.upload_this_machines") }}</option>
            <option value="never">{{ t("remote.do_not_install_there_is") }}</option>
          </select>
        </label>

        <!-- Which machine's key the far-side kernel answers to. This one's by
             default, tunnelled back, so the other machine needs no key of its own
             and no route to the model API. -->
        <label v-if="!isWSL" class="rmtf">
          <span>{{ t("remote.model_credentials") }}</span>
          <select :value="draft.provider || 'local'" :title="t('remote.which_machines_providers_and_keys')" @change="onField('provider', $event)">
            <option value="local">{{ t("remote.this_machines_over_the_tunnel") }}</option>
            <option value="remote">{{ t("remote.that_machines_own") }}</option>
          </select>
        </label>

        <label v-if="!isWSL" class="rmtf rmtck">
          <input type="checkbox" :checked="!!draft.useSSHConfig" @change="onCheck" />
          <span>{{ t("remote.take_anything_left_blank_from") }}</span>
        </label>

        <div class="rmtact">
          <button type="button" data-action="remotes.cancel" @click="closeForm">{{ t("common.cancel") }}</button>
          <button
            type="submit"
            data-go
            :data-action="editing ? 'remotes.save' : 'remotes.add'"
            :disabled="!draft.name.trim() || !!busy || (isWSL && !draft.distro)"
          >
            {{ editing ? t("common.save") : t("common.add") }}
          </button>
        </div>
      </form>
    </template>
  </div>
</template>
