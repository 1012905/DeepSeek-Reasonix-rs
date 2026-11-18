<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, savedNotApplied } from "../../../i18n/kernel";
import type { AgentPort, ShellOption, ShellSettings } from "../../../port/port";
import SaveNote from "../../SaveNote.vue";
import type { SaveNoteState } from "../../SaveNote.vue";

// The product name, not the executable: "powershell.exe" is what the file is
// called, "Windows PowerShell" is what the person installed.
const LABEL: Record<string, string> = {
  bash: "bash",
  "git-bash": "Git Bash",
  pwsh: "PowerShell 7",
  powershell: "Windows PowerShell",
};

// Only the 5.1 line is a warning: it is the one interpreter that refuses syntax
// the model writes by habit.
const NOTE: Record<string, string> = {
  pwsh: "app.understands_and_but_the_syntax",
  powershell: "app.does_not_accept_or_chained",
};

const label = (o: ShellOption) => LABEL[o.name] ?? o.name;

// A typed-in path still has to say which family it belongs to, because that is
// what decides how a command is spelled — and the file name is the only clue on
// offer before it has been run.
const kindOf = (path: string) => (/pwsh/i.test(path) ? "pwsh" : /powershell/i.test(path) ? "powershell" : "bash");

const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const s = ref<ShellSettings | null>(null);
const busy = ref("");
const note = ref<SaveNoteState | null>(null);
const custom = ref("");
const launcher = ref("");

watch(() => props.port, (port) => {
  let live = true;
  s.value = null;
  note.value = null;
  port.shell()
    .then((value) => {
      if (!live) return;
      s.value = value;
      custom.value = value.path ?? "";
      launcher.value = (value.args ?? []).join("\n");
    })
    .catch(() => { if (live) s.value = null; });
  return () => { live = false; };
}, { immediate: true });

const options = computed(() => s.value?.options ?? []);
const auto = computed(() => s.value?.prefer === "auto" && !s.value.path);
const noBash = computed(() => s.value?.platform === "windows" && !options.value.some((o) => o.prefer === "bash"));
const fellBack = computed(() => (s.value?.args?.length ?? 0) > 0 && !s.value?.argv);
// One argument per line: an argument may hold spaces (a distro name), and the
// kernel runs the list as written, with no shell in between.
const launcherArgs = computed(() => launcher.value.split("\n").map((a) => a.trim()).filter((a) => a !== ""));

// Two bashes on one machine are two different programs, so picking one has to
// pin its path. With only one, the name alone survives a PATH that moves.
const pin = (o: ShellOption) => options.value.filter((x) => x.prefer === o.prefer).length > 1;
const picked = (o: ShellOption) => !auto.value && s.value?.effective.path === o.path;

const save = async (what: string, prefer: string, path: string, args?: string[]) => {
  busy.value = what;
  note.value = null;
  try {
    const next = args && args.length > 0
      ? await props.port.saveShell(prefer, path, args)
      : await props.port.saveShell(prefer, path);
    s.value = next;
    custom.value = next.path ?? "";
    launcher.value = (next.args ?? []).join("\n");
    props.onChanged();
  } catch (e) {
    note.value = { text: reason(e), unapplied: savedNotApplied(e) };
  } finally {
    busy.value = "";
  }
};

const saveCustom = () => {
  const path = custom.value.trim();
  if (!path) return;
  void save("custom", launcherArgs.value.length > 0 ? "bash" : kindOf(path), path, launcherArgs.value);
};
</script>

<template>
  <div v-if="!s" class="empty">{{ t("app.could_not_read_the_shell") }}</div>
  <div v-else class="shell">
    <div class="kv">
      <span class="k">{{ t("common.in_effect") }}</span>
      <span class="v">
        {{ label(s.effective) }}{{ s.effective.version ? ` ${s.effective.version}` : "" }} · {{ s.effective.path || "—" }}
      </span>
    </div>

    <div v-if="s.argv" class="kv">
      <span class="k">{{ t("app.launch_command") }}</span>
      <span class="v"><code>{{ `${s.argv.join(" ")} -c …` }}</code></span>
    </div>
    <p v-if="fellBack" class="note" data-tone="warn">{{ t("app.the_saved_launcher_arguments_no") }}</p>

    <!-- One shape for "pick one of a few", the same as every other such choice;
         the executable's path is too long for a segment and is not what you pick
         by anyway — it rides the effective row above. -->
    <div class="seg" data-text role="radiogroup" :aria-label="t('common.which_program_runs_commands')">
      <button type="button" role="radio" data-action="shell.executor" :aria-checked="auto" :disabled="!!busy" @click="void save('auto', 'auto', '')">
        {{ t("common.auto") }}
      </button>
      <button
        v-for="o in options"
        :key="o.path"
        type="button"
        role="radio"
        data-action="shell.executor"
        :data-value="o.prefer"
        :aria-checked="picked(o)"
        :disabled="!!busy"
        @click="void save(o.path, o.prefer, pin(o) ? o.path : '')"
      >
        {{ label(o) }}<i v-if="o.version" class="ver">{{ o.version }}</i>
      </button>
    </div>
    <p class="note">
      <template v-if="auto">{{ t("app.finds_one_itself_preferring_a") }} {{ label(s.auto) }}</template>
      <template v-else>
        <code>{{ s.effective.path }}</code>
        <template v-if="NOTE[s.effective.name]"> · {{ t(NOTE[s.effective.name]) }}</template>
      </template>
    </p>

    <p v-if="noBash" class="note">{{ t("app.there_is_no_bash_on") }}</p>

    <details class="pinpath">
      <summary><span class="fold">{{ t("app.point_at_an_executable") }}</span></summary>
      <p class="note">{{ t("app.a_bash_you_built_yourself") }}</p>
      <div class="fields">
        <label class="grow full">
          <span>{{ t("app.path_to_the_executable") }}</span>
          <input v-model="custom" :placeholder="s.auto.path" :disabled="!!busy" />
        </label>
      </div>
      <div v-if="s.platform === 'windows'" class="fields">
        <label class="grow full">
          <span>{{ t("app.launcher_arguments_optional_one_per") }}</span>
          <textarea v-model="launcher" data-action="shell.custom-args" :rows="4" placeholder="-d&#10;Ubuntu&#10;--&#10;/bin/bash" :disabled="!!busy" />
          <small class="note">{{ t("app.for_example_wsl_exe_needs") }}</small>
        </label>
      </div>
      <div class="acts">
        <button v-if="s.path" class="act" type="button" data-action="shell.custom-path" data-value="clear" :disabled="!!busy" @click="void save('clear', s.prefer, '')">
          {{ t("app.clear_it") }}
        </button>
        <button class="act" type="button" data-action="shell.custom-path" data-primary :disabled="!!busy || !custom.trim()" @click="saveCustom">
          {{ t(busy === "custom" ? "common.verifying" : "app.save_this_path") }}
        </button>
      </div>
    </details>

    <SaveNote :note="note" />
  </div>
</template>
