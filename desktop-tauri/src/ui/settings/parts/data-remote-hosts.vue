<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { say } from "../../../i18n/kernel";
import type { HubPort } from "../../../port/hub";
import type { RemoteHost, RemoteHostEdit, RemoteProbe } from "../../../port/remote";
import { toggleHiddenHost, useHiddenHosts } from "../../../state/remotehide";
import { ROUTE_LABEL, draftOf, statusKey, workspacesOf } from "../../remote/hostbook";
import StudioIcon from "../../StudioIcon.vue";
import RemoteDirs from "./data-remote-dirs.vue";

// This is the settings sheet's host book: the same stored rows the machines
// panel (ui/remote/Remotes.vue) edits, drawn inside a page instead of beside
// one. What it does not carry is that panel's pane work — open, move, close —
// which needs the window's callbacks rather than a settings prop, and what it
// adds is a form field set that panel's compact row does not have room for.
// Everything about what a stored row means is ./remote/hostbook, so the two
// screens cannot drift; only their chrome differs.
//
// The overlap in actions is deliberate and bounded: `remotes.add`, `.add-all`,
// `.save`, `.cancel`, `.kind`, `.remove` name the same intent on both surfaces,
// and the pane-only ones (`remotes.open`, `.close`, `remote-host.move`,
// `remote.more`) exist here in neither spelling. `remote.dirs-*` is the modal's
// and `remotes.dirs-*` this form's, so neither lands on the other.

const props = defineProps<{ hub: HubPort; onError: (e: unknown) => void }>();

const hosts = ref<RemoteHost[] | null>([]);
const candidates = ref<string[]>([]);
const wsl = ref<{ available: boolean; distros: { name: string; version: number; default?: boolean }[] } | null>(null);
const draft = ref<RemoteHostEdit | null>(null);
const dir = ref("");
const editing = ref("");
const confirm = ref("");
const busy = ref("");
const probes = ref<Record<string, RemoteProbe>>({});
const probing = ref("");
const picking = ref(false);
const hidden = useHiddenHosts();
const unavailable = ref(false);

async function load(): Promise<void> {
  try {
    const [book, aliases, distros] = await Promise.all([
      props.hub.remoteHosts(),
      props.hub.remoteCandidates().catch(() => [] as string[]),
      props.hub.wslDistros().catch(() => null),
    ]);
    // Null is a kernel that does not do remote panes at all — a page served to a
    // browser, or a paired device. Nothing here can work, so nothing is drawn.
    if (book === null) { unavailable.value = true; hosts.value = null; return; }
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
  confirm.value = "";
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

// The list is edited whole and saved with the row. Head is the default, so
// promoting one is a move rather than a second field that can disagree.
const setDirs = (next: string[]) => { if (draft.value) draft.value = { ...draft.value, workspaces: next }; };
const addDir = () => {
  const path = dir.value.trim();
  if (!path || !draft.value) return;
  if (!(draft.value.workspaces ?? []).includes(path)) setDirs([...(draft.value.workspaces ?? []), path]);
  dir.value = "";
};

const isWSL = computed(() => draft.value?.kind === "wsl");
const field = (key: keyof RemoteHostEdit, label: string, placeholder?: string) => ({ key, label, placeholder });
const SSH_FIELDS = [
  field("host", "remote.address", "10.0.0.4"),
  field("user", "remote.user", "ada"),
  field("proxyJump", "remote.jump_host", "bastion.example.com"),
  field("identityFile", "remote.private_key_file", "~/.ssh/id_ed25519"),
  field("passphraseEnv", "remote.env_var_holding_the_key", "GPU_BOX_PASSPHRASE"),
  field("passwordEnv", "remote.env_var_with_the_password", "GPU_BOX_PASSWORD"),
] as const;

const set = (key: keyof RemoteHostEdit, value: string | number | boolean) => {
  if (draft.value) draft.value = { ...draft.value, [key]: value } as RemoteHostEdit;
};

function probe(name: string): void {
  probing.value = name;
  props.hub.probeRemote(name)
    .then((rep) => { probes.value = { ...probes.value, [name]: rep }; })
    .catch(props.onError)
    .finally(() => { probing.value = ""; });
}

const closedRoutes = (probe: RemoteProbe) => probe.routes.filter((r) => !r.ok);

// Worded here rather than in the template: `.rmtprobe-r` sets its cells in the
// mono face, and a sentence built inside it would be interface prose drawn as a
// machine fact.
const npmOf = (probe: RemoteProbe): string => probe.npm || t("remote.will_not_run");
const machineOf = (probe: RemoteProbe): string => `${probe.os}/${probe.arch} · ${probe.home}`;
const statusSaid = (host: RemoteHost): string => t(statusKey(host));

const installed = (probe: RemoteProbe): string =>
  probe.kernel
    ? `${probe.kernel}${probe.version ? " " + probe.version : ""}`
    : probe.outdated ? t("remote.it_has_v_there_too", { v: probe.outdated }) : t("remote.none_yet");
</script>

<template>
  <div v-if="unavailable" class="empty">{{ t("kernel.this_kernel_does_not_open") }}</div>
  <div v-else class="rmtbook">
    <div v-if="!draft" class="rmthead">
      <button type="button" class="rmtadd" data-action="remotes.add-ask" @click="draft = draftOf(null)">
        <StudioIcon name="plus" />{{ t("remote.add_a_machine") }}
      </button>
    </div>

    <div v-for="host in hosts ?? []" :key="host.name" class="rmtrow" :data-state="host.status" :data-off="host.disabled ? '' : undefined">
      <span class="rmticon" aria-hidden="true"><StudioIcon name="server" /></span>
      <div class="rmtmain">
        <div class="hd">
          <b>{{ host.name }}</b>
          <span class="tg">{{ host.target }}</span>
          <span class="st" :data-state="host.disabled ? 'off' : host.status">{{ statusSaid(host) }}</span>
        </div>
        <div class="sub">
          <span v-if="host.workspace" dir="ltr">{{ host.workspace }}</span>
          <span v-else class="dim">{{ t("remote.no_default_workspace") }}</span>
          <span v-if="workspacesOf(host).length > 1" class="tag">{{ t("remote.n_more_projects", { n: workspacesOf(host).length - 1 }) }}</span>
          <span v-if="host.kind === 'wsl'" class="tag">{{ t("remote.wsl_distribution") }}</span>
          <span v-if="host.useSSHConfig" class="tag">{{ t("remote.follows_ssh_config") }}</span>
          <span v-if="host.forwards" class="tag">{{ t("remote.n_forwards", { n: host.forwards }) }}</span>
          <span v-if="host.panes" class="tag">{{ t("remote.n_panes", { n: host.panes }) }}</span>
        </div>
      </div>
      <div class="rmtside">
        <button
          v-if="host.status !== 'connected' && !host.disabled"
          type="button"
          class="rmtnext"
          data-action="remote.probe"
          :data-target="host.name"
          :disabled="probing === host.name"
          @click="probe(host.name)"
        >{{ t(probing === host.name ? "remote.asking" : "common.test_it") }}</button>
        <button
          type="button"
          class="rmthost-btn"
          data-action="remote.edit"
          :data-target="host.name"
          :aria-label="t('remote.more_for_name', { name: host.name })"
          :title="t('common.more')"
          @click="confirm = ''; editing = host.name; draft = draftOf(host)"
        ><StudioIcon name="more" /></button>
      </div>
      <!-- The row's own controls, all of which rewrite the whole entry. -->
      <div class="rmtmenu" role="group" :aria-label="t('remote.more_for_name', { name: host.name })">
        <button type="button" role="menuitem" data-action="remote.enable" :data-target="host.name" :disabled="busy === host.name" @click="save({ ...draftOf(host), disabled: !host.disabled })">
          {{ t(host.disabled ? "remote.turn_this_machine_on" : "remote.turn_this_machine_off") }}
        </button>
        <button type="button" role="menuitem" data-action="remote.hide" :data-target="host.name" @click="toggleHiddenHost(host.name)">
          {{ t(hidden.includes(host.name) ? "remote.show_in_the_rail" : "remote.hide_from_the_rail") }}
        </button>
        <button type="button" role="menuitem" data-danger="" data-action="remote.remove-ask" :data-target="host.name" :disabled="!!busy" @click="confirm = host.name">
          {{ t("common.remove") }}
        </button>
      </div>
      <div v-if="probes[host.name]" class="rmtprobe" :data-ok="probes[host.name].ready ? '' : undefined">
        <!-- The values sit in `code` so the mono face is on the machine string
             alone, and the row that also carries a label never reads as one. -->
        <div class="rmtprobe-r">
          <span class="k">{{ t("remote.machine") }}</span>
          <span class="v"><code>{{ machineOf(probes[host.name]) }}</code></span>
        </div>
        <div class="rmtprobe-r">
          <span class="k">reasonix</span>
          <span class="v"><code>{{ installed(probes[host.name]) }}</code></span>
        </div>
        <div class="rmtprobe-r">
          <span class="k">npm</span>
          <span class="v"><code>{{ npmOf(probes[host.name]) }}</code></span>
        </div>
        <p v-for="route in closedRoutes(probes[host.name])" :key="route.name" class="rmtprobe-why">
          {{ say({ code: route.code, params: { host: host.name } }, t("remote.the_name_route_is_closed", { name: t(ROUTE_LABEL[route.name] ?? route.name) })) }}
        </p>
        <div class="rmtprobe-end">
          {{ t(probes[host.name].ready ? "remote.ready_there_is_a_kernel" : "remote.not_ready_clearing_any_one") }}
        </div>
      </div>
      <div v-if="confirm === host.name" class="rmtconfirm" role="alertdialog">
        <span>{{ t("remote.remove_name_from_the_list", { name: host.name }) }}</span>
        <button type="button" data-action="remote.remove-cancel" @click="confirm = ''">{{ t("common.cancel") }}</button>
        <button type="button" data-danger="" data-action="remotes.remove" :data-target="host.name" @click="drop(host.name)">{{ t("common.remove") }}</button>
      </div>
    </div>

    <p v-if="!(hosts ?? []).length && !draft" class="rmtempty">{{ t("remote.no_remote_machines_yet_add") }}</p>

    <!-- Importing beats typing: on a machine that already uses ssh the addresses
         are written down next door, and this only borrows the name. -->
    <div v-if="candidates.length > 0" class="rmtcands">
      <span class="cap">{{ t("remote.also_in_ssh_config") }}</span>
      <button
        v-for="alias in candidates"
        :key="alias"
        type="button"
        class="rmtcand"
        data-action="remotes.add"
        :disabled="!!busy"
        :title="t('remote.add_it_with_the_settings')"
        @click="save({ name: alias, useSSHConfig: true })"
      ><StudioIcon name="plus" /> {{ alias }}</button>
      <!-- A sentence rather than an alias, so it is not drawn in the mono face
           the alias chips use. -->
      <button
        v-if="candidates.length > 1"
        type="button"
        class="spill"
        data-all=""
        data-action="remotes.add-all"
        :disabled="!!busy"
        @click="(async () => { for (const alias of candidates) await save({ name: alias, useSSHConfig: true }); })()"
      >{{ t("remote.add_them_all") }}</button>
    </div>

    <RemoteDirs
      v-if="picking && editing"
      :hub="hub"
      :host="editing"
      :start="(draft?.workspaces ?? [])[0] ?? ''"
      :on-close="() => { picking = false; }"
      :on-pick="(path) => { picking = false; if (!(draft?.workspaces ?? []).includes(path)) setDirs([...(draft?.workspaces ?? []), path]); }"
    />

    <div v-if="draft" class="rmtform">
      <label class="rmtf">
        <span>{{ t("common.name") }}</span>
        <input data-action="remotes.draft" data-value="name" :value="draft.name" placeholder="gpu-box" :disabled="!!editing" @input="set('name', ($event.target as HTMLInputElement).value)" />
      </label>
      <!-- A saved machine keeps the way it is reached: the two kinds share no
           field, so turning one into the other is a new machine, not an edit. -->
      <div v-if="wsl && !editing" class="rmtf" role="radiogroup" :aria-label="t('remote.connection_type')">
        <span>{{ t("remote.connection_type") }}</span>
        <div class="rmtkinds">
          <button type="button" role="radio" data-action="remotes.kind" data-value="ssh" :aria-checked="(draft.kind ?? '') === ''" @click="set('kind', '')">{{ t("remote.ssh_host") }}</button>
          <button
            type="button"
            role="radio"
            data-action="remotes.kind"
            data-value="wsl"
            :aria-checked="draft.kind === 'wsl'"
            @click="set('kind', 'wsl'); if (!draft.distro) set('distro', wsl?.distros.find((x) => x.version === 2)?.name ?? '')"
          >{{ t("remote.wsl_distribution") }}</button>
        </div>
      </div>
      <label v-if="isWSL" class="rmtf">
        <span>{{ t("remote.distribution") }}</span>
        <select data-action-change="remotes.distro" :value="draft.distro ?? ''" :disabled="!!editing" @change="set('distro', ($event.target as HTMLSelectElement).value)">
          <option v-if="editing && !(wsl?.distros ?? []).some((x) => x.name === draft?.distro)" :value="draft.distro">{{ draft.distro }}</option>
          <option v-for="x in wsl?.distros ?? []" :key="x.name" :value="x.name" :disabled="x.version !== 2">
            {{ x.version === 2 ? x.name : t("remote.name_wsl_1_not_supported", { name: x.name }) }}
          </option>
        </select>
      </label>
      <label v-for="f in isWSL ? [] : SSH_FIELDS" :key="f.key" class="rmtf">
        <span>{{ t(f.label) }}</span>
        <input
          data-action="remotes.draft"
          :data-value="f.key"
          :value="String(draft[f.key] ?? '')"
          :placeholder="f.placeholder"
          :spellcheck="false"
          @input="set(f.key, ($event.target as HTMLInputElement).value)"
        />
      </label>
      <label v-if="!isWSL" class="rmtf">
        <span>{{ t("common.port") }}</span>
        <input
          data-action="remotes.draft"
          data-value="port"
          inputmode="numeric"
          :value="draft.port ? String(draft.port) : ''"
          placeholder="22"
          @input="set('port', Number(($event.target as HTMLInputElement).value.replace(/\D/g, '')) || 0)"
        />
      </label>
      <p v-if="isWSL" class="rmthint">{{ t("remote.the_kernel_runs_in_a") }}</p>

      <!-- Same shape the sandbox's writable list uses: one row per folder, then
           what can be done to it. The head carries the default badge rather than
           a separate field, because it is the same folder. -->
      <div class="rmtdirs">
        <div class="sublb">{{ t("remote.projects_on_this_machine") }}</div>
        <div v-for="(path, i) in draft.workspaces ?? []" :key="path" class="prule">
          <code dir="ltr">{{ path }}</code>
          <span v-if="i === 0" class="tag">{{ t("common.default") }}</span>
          <button v-else type="button" class="act ghost" data-action="remotes.make-default" :data-target="path" :aria-label="t('remote.make_path_the_default', { path })" @click="setDirs([path, ...(draft.workspaces ?? []).filter((x) => x !== path)])">
            {{ t("common.make_default") }}
          </button>
          <button type="button" class="act ghost" data-action="remotes.unlist-dir" :data-target="path" :aria-label="t('remote.stop_listing_path', { path })" @click="setDirs((draft.workspaces ?? []).filter((x) => x !== path))">
            {{ t("common.delete") }}
          </button>
        </div>
        <div class="prule" data-add="">
          <input
            :value="dir"
            :placeholder="t('remote.a_project_directory_over_there')"
            :aria-label="t('remote.a_project_directory_over_there')"
            @input="dir = ($event.target as HTMLInputElement).value"
            @keydown.enter="addDir"
          />
          <button type="button" class="act" data-action="remotes.list-dir" :disabled="!dir.trim()" @click="addDir">{{ t("common.add") }}</button>
          <button v-if="editing" type="button" class="act ghost" data-action="remotes.browse" @click="picking = true">{{ t("remote.browse") }}</button>
        </div>
      </div>

      <p v-if="!isWSL" class="rmthint">{{ t("remote.left_blank_it_tries_in") }}</p>

      <!-- The kernel names this setting when an install fails, so it has to be
           here: an error saying "install it with npm instead" beside a setting
           nobody can reach says nothing. -->
      <label class="rmtf">
        <span>{{ t("remote.installing") }}</span>
        <select data-action="remotes.draft" data-value="serve-install" :value="draft.serveInstall || 'auto'" :title="t('remote.a_first_connect_installs_a')" @change="set('serveInstall', ($event.target as HTMLSelectElement).value)">
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
        <select data-action="remotes.draft" data-value="provider" :value="draft.provider || 'local'" :title="t('remote.which_machines_providers_and_keys')" @change="set('provider', ($event.target as HTMLSelectElement).value)">
          <option value="local">{{ t("remote.this_machines_over_the_tunnel") }}</option>
          <option value="remote">{{ t("remote.that_machines_own") }}</option>
        </select>
      </label>
      <label v-if="!isWSL" class="rmtf rmtck">
        <input type="checkbox" data-action="remotes.draft" data-value="use-ssh-config" :checked="!!draft.useSSHConfig" @change="set('useSSHConfig', ($event.target as HTMLInputElement).checked)" />
        <span>{{ t("remote.take_anything_left_blank_from") }}</span>
      </label>
      <div class="rmtact">
        <button type="button" data-action="remotes.cancel" @click="draft = null; editing = ''; dir = ''">{{ t("common.cancel") }}</button>
        <button
          type="button"
          data-go=""
          :data-action="editing ? 'remotes.save' : 'remotes.add'"
          :disabled="!draft.name.trim() || !!busy || (isWSL && !draft.distro)"
          @click="save(draft)"
        >{{ t(editing ? "common.save" : "common.add") }}</button>
      </div>
    </div>
  </div>
</template>
