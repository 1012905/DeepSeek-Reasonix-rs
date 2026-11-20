<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, SkillCatalog, SkillEntry } from "../../../port/port";
import Switch from "../../Switch.vue";
import SkillDeclarations from "../../SkillDeclarations.vue";
import SkillRemove from "../../SkillRemove.vue";
import SkillSpecInfo from "../../SkillSpecInfo.vue";

// Searching skills is `skills.search`, not `extensions.search`: that one is the
// package list's box (./ext-plugins.vue, and Packages.tsx before it), and the two
// filters read different catalogs on different endpoints. One id for both would
// make a landing or a query land on whichever page happened to be mounted.
//
// Where a skill came from, as the list groups it. An unknown scope keeps its own
// name rather than being folded into one of these.
const SCOPE: Record<string, string> = { global: "ext.mine", project: "common.project", custom: "common.custom", builtin: "common.built_in" };

// Which way the model can reach this skill, and whether it can reach it at all.
// An "auto" skill with model discovery off is manual in practice, and the row has
// to say so: the switch is a different fact from the reach.
function reach(sk: SkillEntry, implicit: boolean): { key: string; reachable: boolean } {
  if (!sk.enabled) return { key: "", reachable: true };
  const auto = !sk.manual && implicit;
  if (sk.slashName && auto) return { key: "", reachable: true };
  if (sk.slashName) return { key: "ext.by_name_only", reachable: true };
  if (auto) return { key: "ext.models_choice_only", reachable: true };
  return { key: "ext.unreachable", reachable: false };
}

// Only the states worth interrupting a list for: in step and "cannot tell" draw
// nothing, because a badge on every row is a badge on none.
function drift(source: SkillEntry["source"]): { key: string; says: string } | null {
  switch (source?.state) {
    case "source_changed":
      return { key: "ext.source_has_changed", says: t("ext.the_source_folder_has_newer", { path: source.path }) };
    case "local_edited":
      return { key: "ext.copy_was_edited", says: t("ext.the_installed_copy_differs_from", { path: source.path }) };
    case "diverged":
      return { key: "ext.both_changed", says: t("ext.the_installed_copy_and_the", { path: source.path }) };
    case "source_missing":
      return { key: "ext.source_folder_is_gone", says: t("ext.the_source_folder_cannot_be", { path: source.path }) };
  }
  return null;
}

const props = defineProps<{ port: AgentPort; root: string; onChanged: () => void }>();

const cat = ref<SkillCatalog | null>(null);
const busy = ref("");
const failedNote = ref("");
const query = ref("");
const removing = ref<SkillEntry | null>(null);

const load = async (port: AgentPort) => {
  try {
    cat.value = await port.skills(props.root || undefined);
    failedNote.value = "";
  } catch (e) {
    cat.value = null;
    failedNote.value = reason(e);
  }
};

watch([() => props.port, () => props.root], () => { cat.value = null; void load(props.port); }, { immediate: true });

const implicit = computed(() => cat.value?.implicit ?? true);
const needle = computed(() => query.value.trim().toLowerCase());
const rows = computed(() => (cat.value?.skills ?? []).filter((sk) => {
  if (!needle.value) return true;
  return [sk.name, sk.slashName, sk.description, sk.plugin].some((field) => field?.toLowerCase().includes(needle.value));
}));
const on = computed(() => rows.value.filter((sk) => sk.enabled).length);

// Grouped by where the skill came from, in the order a reader looks for one:
// their own machine, this project, a package, then what shipped with the app.
const groups = computed(() => {
  const order = ["global", "project", "custom", "builtin"];
  const by = new Map<string, SkillEntry[]>();
  for (const sk of rows.value) {
    const key = sk.scope ?? "";
    by.set(key, [...(by.get(key) ?? []), sk]);
  }
  return [...by.entries()].sort((a, b) => {
    const ai = order.indexOf(a[0]);
    const bi = order.indexOf(b[0]);
    return (ai < 0 ? order.length : ai) - (bi < 0 ? order.length : bi);
  }).map(([scope, list]) => ({ scope, label: SCOPE[scope] ? t(SCOPE[scope]) : scope || t("ext.other"), list }));
});

// Flipping a skill this project inherits writes a project row by default: the
// person answered for this folder, and two projects may hold skills of one name.
const toggle = async (sk: SkillEntry) => {
  busy.value = `toggle:${sk.name}`;
  failedNote.value = "";
  try {
    await props.port.setSkillEnabled(sk.name, !sk.enabled, "project", props.root || undefined);
    props.onChanged();
    await load(props.port);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

// Drops this project's exception so the skill inherits again.
const clear = async (sk: SkillEntry) => {
  busy.value = `clear:${sk.name}`;
  failedNote.value = "";
  try {
    await props.port.clearSkillOverride(sk.name, props.root || undefined);
    props.onChanged();
    await load(props.port);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

// A built-in or a package's skill is refused by the kernel; the button is only
// drawn where there is a file to delete.
const removable = (sk: SkillEntry) => !props.root && !!sk.removes;
</script>

<template>
  <div v-if="!cat" class="empty">{{ failedNote || t("ext.could_not_read_name", { name: t("settings.skills") }) }}</div>
  <div v-else class="box">
    <div v-if="rows.length > 0" class="msearch">
      <input v-model="query" type="search" spellcheck="false" data-action="skills.search" :aria-label="t('ext.search_installed_packages')" :placeholder="t('ext.search_package_names_skills_commands')" />
      <button v-if="query" class="act ghost" type="button" data-action="skills.clear-search" @click="query = ''">{{ t("common.clear_search") }}</button>
      <span class="cnt" aria-live="polite" aria-atomic="true">{{ t("settings.on_all_on", { on: on, all: rows.length }) }}</span>
    </div>
    <div v-if="rows.length === 0" class="empty">{{ t(query ? "ext.no_matching_tool" : "settings.no_skills_in_this_working") }}</div>

    <!-- One command is what gets typed for a skill, so it is what the row names
         first; a skill with no slash name is reachable only through the model. -->
    <section v-for="g in groups" :key="g.scope" class="rgroup" :aria-label="g.label">
      <h4 class="hd">{{ g.label }}</h4>
      <div v-for="sk in g.list" :key="sk.name" class="skrow" :data-off="sk.enabled ? undefined : ''" :data-local="sk.switchScope === 'project' ? '' : undefined">
        <span class="sk-main">
          <span class="sk-line">
            <span class="nm">{{ sk.slashName ? `/${sk.slashName}` : sk.name }}</span>
            <i v-if="drift(sk.source)" class="w" :data-drift="sk.source?.state" :title="drift(sk.source)!.says">{{ t(drift(sk.source)!.key) }}</i>
            <i v-if="sk.subagent" class="ro">{{ t("common.subagents") }}</i>
            <i v-if="sk.readOnly" class="ro">{{ t("common.read_only") }}</i>
            <span class="how"><i v-if="reach(sk, implicit).key" :class="reach(sk, implicit).reachable ? 'w' : 'w none'">{{ t(reach(sk, implicit).key) }}</i></span>
          </span>
          <span class="ds" :title="sk.description || undefined">{{ sk.description || t("ext.no_description_written") }}</span>
        </span>
        <span v-if="sk.switchScope === 'project'" class="exc" :title="t('ext.go_back_to_following_the')">{{ t("ext.this_project_only_2") }}</span>
        <span class="sc" :title="sk.path">{{ sk.plugin || t(SCOPE[sk.scope ?? ""] ?? "") || sk.scope }}</span>
        <Switch
          data-action="skill.enabled"
          :data-target="sk.name"
          :on="sk.enabled"
          :busy="busy === `toggle:${sk.name}` || busy === `clear:${sk.name}`"
          :label="t(sk.enabled ? 'common.disable_name' : 'common.enable_name', { name: sk.name })"
          :on-click="() => void toggle(sk)"
        />
        <SkillDeclarations :sk="sk" />
        <SkillSpecInfo :sk="sk" />
        <span v-if="removable(sk) || sk.switchScope === 'project'" class="acts">
          <button v-if="sk.switchScope === 'project'" class="act ghost" type="button" :disabled="!!busy" data-action="skill.clear-override" :data-target="sk.name" @click="void clear(sk)">
            {{ t("ext.go_back_to_following_the") }}
          </button>
          <button v-if="removable(sk)" class="act ghost" type="button" data-action="skills.remove-open" :data-target="sk.name" :disabled="!!busy" @click="removing = sk">
            {{ t("ext.remove_2") }}
          </button>
        </span>
      </div>
    </section>

    <SkillRemove v-if="removing" :port="port" :sk="removing" :on-close="() => (removing = null)" :on-done="() => { removing = null; onChanged(); void load(port); }" />

    <div v-if="failedNote" class="why" role="alert">{{ failedNote }}</div>
  </div>
</template>
