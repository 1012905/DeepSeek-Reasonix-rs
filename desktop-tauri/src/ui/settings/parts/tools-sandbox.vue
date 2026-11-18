<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, say } from "../../../i18n/kernel";
import type { AgentPort, SandboxSettings } from "../../../port/port";
import Switch from "../../Switch.vue";

// Two questions, in the order they matter: how far a write reaches, and whether
// the command that makes it runs jailed. The first is always in force; the
// second needs an OS sandbox and says so where there is none.
const MODES: readonly (readonly [string, string])[] = [
  ["enforce", "safety.jailed"],
  ["off", "safety.unconfined"],
];

const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const box = ref<SandboxSettings | null>(null);
const root = ref("");
const draft = ref("");
const busy = ref("");
const error = ref("");

watch(() => props.port, (port) => {
  let live = true;
  box.value = null;
  error.value = "";
  port.sandbox()
    .then((value) => {
      if (!live) return;
      box.value = value;
      root.value = value.workspaceRoot;
    })
    .catch(() => { if (live) box.value = null; });
  return () => { live = false; };
}, { immediate: true });

const save = async (what: string, next: SandboxSettings) => {
  busy.value = what;
  error.value = "";
  try {
    const saved = await props.port.saveSandbox(next);
    box.value = saved;
    root.value = saved.workspaceRoot;
    props.onChanged();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const commitRoot = () => {
  if (!box.value || root.value === box.value.workspaceRoot) return;
  void save("root", { ...box.value, workspaceRoot: root.value });
};

const pick = async () => {
  const path = await props.port.pickFolder().catch(() => null);
  if (path && box.value) void save("root", { ...box.value, workspaceRoot: path });
};

const addRoot = () => {
  const path = draft.value.trim();
  if (!box.value || !path) return;
  void save("extra", { ...box.value, allowWrite: [...box.value.allowWrite, path] });
  draft.value = "";
};

// Rendered from what the confiner will actually do, not from the word in the
// file: an unset mode enforces everywhere but Windows, which runs unconfined
// however the file reads. Drawing the file would put the wrong posture on the
// one screen that reports it.
const mode = computed(() => box.value?.effectiveBash || box.value?.bash || "off");
const written = computed(() => (box.value?.bash ?? "").trim());
const overruled = computed(() => written.value !== "" && written.value !== mode.value);
const why = computed(() => say({ code: box.value?.whyCode, error: box.value?.why }, ""));
const label = (id: string) => t(MODES.find(([m]) => m === id)?.[1] ?? id);

const modeChoice = computed(() => box.value ? MODES.map(([id, name]) => ({
  id,
  name: t(name),
  locked: id === "enforce" && !box.value!.available,
})) : []);
</script>

<template>
  <div v-if="!box" class="empty">{{ t("safety.cannot_read_the_sandbox_config") }}</div>
  <div v-else class="box">
    <div v-if="box.shadowedBy" class="find" data-lvl="warn" role="status">
      <span class="t">{{ t("safety.this_project_ships_its_own_2") }}</span>
      <span class="why">{{ t("safety.path_declares_a_sandbox_too", { path: box.shadowedBy }) }}</span>
    </div>

    <div class="sec">
      <h3>{{ t("safety.where_writes_may_land") }}</h3>
      <p class="note">{{ t("safety.even_an_approved_write_can") }}</p>
      <div class="fields">
        <label class="grow">
          <span>{{ t("safety.workspace_root") }}</span>
          <input
            v-model="root"
            data-action="sandbox.workspace-root"
            :placeholder="t('safety.empty_the_sessions_own_directory')"
            :disabled="!!busy"
            @blur="commitRoot"
            @keydown.enter="commitRoot"
          />
        </label>
        <button class="act" type="button" data-action="sandbox.workspace-root" data-value="browse" :disabled="!!busy" @click="void pick()">
          {{ t("common.choose_a_folder") }}
        </button>
      </div>

      <div class="extra">
        <div class="sublb">{{ t("safety.also_writable") }}</div>
        <div v-for="p in box.allowWrite" :key="p" class="prule">
          <code>{{ p }}</code>
          <button
            class="act ghost"
            type="button"
            data-action="sandbox.remove-write-root"
            :data-target="p"
            :disabled="!!busy"
            :aria-label="t('safety.stop_allowing_writes_to_path', { path: p })"
            @click="void save('extra', { ...box, allowWrite: box.allowWrite.filter((x) => x !== p) })"
          >{{ t("common.delete") }}</button>
        </div>
        <!-- Same row shape as the entries above: one thing, then what you can do
             to it. A differently built add box reads as a third kind of control. -->
        <div class="prule" data-add="">
          <input
            v-model="draft"
            data-action="sandbox.add-write-root"
            :placeholder="t('safety.open_one_more_writable_directory')"
            :disabled="!!busy"
            @keydown.enter="addRoot"
          />
          <button class="act" type="button" data-action="sandbox.add-write-root" :disabled="!!busy || !draft.trim()" @click="addRoot">
            {{ t("common.add") }}
          </button>
        </div>
      </div>

      <!-- The expansion, not the spelling: an empty root above is still a real
           directory down here, and ${VAR} in a path is answered rather than echoed. -->
      <div class="kv">
        <span class="k">{{ t("safety.actually_writable") }}</span>
        <span class="v">
          <template v-if="box.effectiveWriteRoots.length">
            <code v-for="r in box.effectiveWriteRoots" :key="r">{{ r }}</code>
          </template>
          <template v-else>—</template>
        </span>
      </div>

      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("safety.protect_changed_files") }}</span>
          <span class="ds">{{ t("safety.if_a_file_the_model") }}</span>
        </span>
        <Switch
          data-action="sandbox.protect-changed-files"
          :on="box.protectChangedFiles"
          :busy="busy === 'protect'"
          :label="t('safety.protect_changed_files')"
          :on-click="() => void save('protect', { ...box!, protectChangedFiles: !box!.protectChangedFiles })"
        />
      </div>
    </div>

    <div class="sec">
      <h3>{{ t("safety.how_commands_run") }}</h3>
      <p v-if="box.available && box.platform === 'windows'" class="note">{{ t("safety.on_windows_once_on_commands") }}</p>
      <p v-else-if="box.available" class="note">{{ t("safety.inside_the_sandbox_a_command") }}</p>
      <div v-else class="find" data-lvl="warn" role="status">
        <span class="t">{{ t("safety.this_machine_has_no_usable") }}</span>
        <span class="why">{{ why || t("safety.commands_run_unconfined_the_writable") }}</span>
      </div>
      <!-- A host that can never jail still shows the option, because this is the
           only place a reader learns the mode exists and why it is out. -->
      <div class="seg" data-text role="radiogroup" :aria-label="t('safety.how_commands_run')">
        <button
          v-for="m in modeChoice"
          :key="m.id"
          type="button"
          role="radio"
          data-action="sandbox.mode"
          :data-value="m.id"
          :aria-checked="mode === m.id"
          :disabled="!!busy || m.locked"
          :data-locked="m.locked ? '' : undefined"
          :title="m.locked ? why : undefined"
          @click="void save('bash', { ...box!, bash: m.id })"
        >{{ m.name }}</button>
      </div>
      <div v-if="overruled" class="kv">
        <span class="k">{{ t("common.actually_in_force") }}</span>
        <span class="v">{{ t("safety.configured_as_want_running_as", { want: label(written), now: label(mode) }) }}</span>
      </div>
      <div v-if="mode === 'enforce'" class="lrow">
        <span class="tx">
          <span class="lb">{{ t("safety.allow_network_inside_the_sandbox") }}</span>
          <span class="ds">{{ t("safety.with_this_off_installing_dependencies") }}</span>
        </span>
        <Switch
          data-action="sandbox.network"
          :on="box.network"
          :busy="busy === 'net'"
          :label="t('safety.allow_network_inside_the_sandbox')"
          :on-click="() => void save('net', { ...box!, network: !box!.network })"
        />
      </div>
    </div>

    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
