<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, PluginAction, PluginExport, PluginInstallRequest, PluginItem, PluginPackage, PluginPlan } from "../../../port/port";
import Switch from "../../Switch.vue";
import CopyButton from "../../CopyButton.vue";
import ModPreviewBlock from "../../ModPreviewBlock.vue";

// What a package brought is not one number: skills and commands are menu
// entries, while hooks, servers and a runtime are code that runs. Counting them
// together would let the second hide inside the first.
const KIND_TITLE: Record<string, string> = {
  high: "ext.these_run_programs_on_this",
  medium: "ext.these_change_what_is_available",
  low: "ext.these_only_add_files",
};
const ORDER = ["high", "medium", "low"] as const;

const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const packages = ref<PluginPackage[] | null>(null);
const failedNote = ref("");
const busy = ref("");
const q = ref("");
// Adding and updating are the same door from two sides: an update skips the box
// because its source was recorded at install, and reads the plan against what is
// already there — a new version's new hook is the thing worth seeing.
const adding = ref<"" | "source" | "folder">("");
const draftText = ref("");
const plan = ref<PluginPlan | null>(null);
const done = ref<PluginPlan | null>(null);
const updating = ref<PluginPackage | null>(null);
const confirming = ref("");
const exported = ref<PluginExport | null>(null);

const load = async (port: AgentPort) => {
  try {
    packages.value = await port.plugins();
    failedNote.value = "";
  } catch (e) {
    packages.value = null;
    failedNote.value = reason(e);
  }
};

watch(() => props.port, (port) => {
  packages.value = null;
  failedNote.value = "";
  void load(port);
}, { immediate: true });

const rows = computed(() => {
  const query = q.value.trim().toLowerCase();
  return (packages.value ?? []).filter((p) => matches(p, query));
});

function matches(p: PluginPackage, query: string): boolean {
  if (!query) return true;
  const fields = [
    p.name,
    ...[p.skills, p.commands, p.agents, p.prompts, p.themes].flatMap((items) =>
      items?.flatMap((item) => [item.name, item.invocation, item.description]) ?? []),
    ...(p.mcpServers?.flatMap((server) => [server.name, server.displayName, server.description, server.command, server.url, server.transport]) ?? []),
    ...(p.hooks?.flatMap((hook) => [hook.event, hook.description, hook.command, hook.contextFile]) ?? []),
    p.runtime && [p.runtime.command, ...(p.runtime.args ?? [])].join(" "),
    ...(p.runtime?.tools ?? []),
  ];
  return fields.some((field) => field?.toLowerCase().includes(query));
}

const summary = (p: PluginPackage): string => {
  const parts: string[] = [];
  const add = (n: number | undefined, unit: string) => { if (n) parts.push(t(unit, { n })); };
  add(p.skills?.length, "ext.n_skills");
  add(p.commands?.length, "ext.n_commands");
  add(p.agents?.length, "subagent.group.count");
  add(p.prompts?.length, "ext.n_prompts");
  add(p.themes?.length, "ext.n_palettes");
  add(p.hooks?.length, "ext.n_hooks");
  add(p.mcpServers?.length, "settings.n_services");
  if (p.runtime) parts.push(t("ext.long_running_process"));
  return parts.join(" · ");
};

const setEnabled = async (p: PluginPackage) => {
  busy.value = `toggle:${p.name}`;
  failedNote.value = "";
  try {
    const out = await props.port.setPluginEnabled(p.name, !p.enabled);
    if (out.reloadError) failedNote.value = out.reloadError;
    else props.onChanged();
    await load(props.port);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const exportOne = async (p: PluginPackage) => {
  busy.value = `export:${p.name}`;
  exported.value = null;
  try {
    exported.value = await props.port.exportPlugin(p.name);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const removeOne = async (p: PluginPackage) => {
  busy.value = `remove:${p.name}`;
  failedNote.value = "";
  try {
    const out = await props.port.removePlugin(p.name);
    confirming.value = "";
    if (out.applied) {
      if (out.reloadError) failedNote.value = out.reloadError;
      else props.onChanged();
    }
    if (!out.ok) failedNote.value = out.error || out.next || t("ext.it_could_not_be_removed");
    await load(props.port);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const request = (planId?: string): PluginInstallRequest => ({
  source: updating.value ? updating.value.source ?? "" : draftText.value.trim(),
  name: updating.value?.name,
  replace: !!updating.value,
  planId,
});

const look = async () => {
  if (!request().source || busy.value) return;
  busy.value = "look";
  failedNote.value = "";
  try {
    const next = await props.port.planPlugin(request());
    // "Nothing installable here" is the plan's own answer, and its reason is
    // more useful than anything this panel could invent.
    if (!next.actions?.length) {
      failedNote.value = next.error || next.next || t("ext.there_is_nothing_installable_at");
      plan.value = null;
      return;
    }
    plan.value = next;
  } catch (e) {
    plan.value = null;
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const install = async () => {
  if (!plan.value) return;
  busy.value = "install";
  failedNote.value = "";
  try {
    done.value = await props.port.installPlugin(request(plan.value.planId));
    if (done.value.applied) await load(props.port);
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const startAdd = async (way: "source" | "folder") => {
  updating.value = null;
  plan.value = null;
  done.value = null;
  draftText.value = "";
  failedNote.value = "";
  adding.value = way;
  if (way === "folder") {
    const dir = await props.port.pickFolder().catch(() => null);
    if (dir) {
      draftText.value = dir;
      await look();
    }
  }
};

const startUpdate = (p: PluginPackage) => {
  if (!p.source) return;
  adding.value = "";
  draftText.value = "";
  done.value = null;
  confirming.value = "";
  updating.value = p;
  void look();
};

const close = () => {
  adding.value = "";
  updating.value = null;
  plan.value = null;
  done.value = null;
  draftText.value = "";
  failedNote.value = "";
};

const groups = computed(() => ORDER
  .map((level) => ({ level, actions: (plan.value?.actions ?? []).filter((a) => (a.riskLevel || "low") === level) }))
  .filter((g) => g.actions.length > 0));

// What this version brings that the installed one did not. A package that was
// only skills last time and starts a process this time is the case the whole
// confirmation exists for.
const gained = computed(() => {
  const current = updating.value;
  const next = plan.value?.actions?.find((a) => a.kind === "plugin");
  if (!current || !next) return [];
  const out: string[] = [];
  if (next.runtime && !current.runtime) out.push(t("ext.a_long_running_process"));
  const tools = (next.runtime?.tools ?? []).filter((name) => !current.runtime?.tools?.includes(name));
  if (tools.length > 0) out.push(t("ext.tools_names", { names: tools.join("、") }));
  if ((next.hookCount ?? 0) > (current.hooks?.length ?? 0)) out.push(t("ext.n_hooks", { n: (next.hookCount ?? 0) - (current.hooks?.length ?? 0) }));
  if ((next.toolCount ?? 0) > (current.mcpServers?.length ?? 0)) out.push(t("ext.n_external_services", { n: (next.toolCount ?? 0) - (current.mcpServers?.length ?? 0) }));
  return out;
});

const contributes = (a: PluginAction): string[] => {
  const parts: string[] = [];
  const add = (n: number | undefined, key: string) => { if (n) parts.push(t(key, { n })); };
  add(a.skillCount, "ext.n_skills");
  add(a.commandCount, "ext.n_commands");
  add(a.agentCount, "subagent.group.count");
  add(a.promptCount, "ext.n_prompts");
  add(a.themeCount, "ext.n_palettes");
  return parts;
};

// What will execute, one row per kind: a process, a hook and an external server
// are three different decisions.
const executes = (a: PluginAction): { label: string; detail: string; why?: string }[] => {
  const out: { label: string; detail: string; why?: string }[] = [];
  if (a.runtime) out.push({ label: "ext.long_running_process", detail: [a.runtime.command, ...(a.runtime.args ?? [])].join(" "), why: "ext.runs_inside_reasonix_can_read" });
  if (a.runtime?.tools?.length) out.push({ label: "ext.new_tools", detail: a.runtime.tools.join("、"), why: "ext.the_agent_can_call_them" });
  if (a.hookCount) out.push({ label: "ext.automatic_hooks", detail: t("ext.n_hooks", { n: a.hookCount }), why: "ext.run_automatically_during_the_session" });
  if (a.toolCount) out.push({ label: "ext.external_services", detail: t("ext.n_mcp_services", { n: a.toolCount }), why: "ext.provides_capabilities_equivalent_to_the" });
  if (a.kind === "mcp" && a.command) out.push({ label: "ext.will_start", detail: [a.command, ...(a.args ?? [])].join(" ") });
  if (a.kind === "mcp" && a.url) out.push({ label: "ext.will_connect_to", detail: a.url });
  return out;
};

const version = computed(() => plan.value?.actions?.find((a) => a.kind === "plugin")?.version ?? "");
</script>

<template>
  <div v-if="packages === null" class="empty">{{ failedNote || t("ext.could_not_read_name", { name: t("settings.plugin_packages") }) }}</div>
  <div v-else class="box">
    <div v-if="packages.length > 0" class="msearch">
      <input v-model="q" type="search" spellcheck="false" data-action="extensions.search" :aria-label="t('ext.search_installed_packages')" :placeholder="t('ext.search_package_names_skills_commands')" />
      <button v-if="q" class="act ghost" type="button" data-action="extensions.clear-search" @click="q = ''">{{ t("common.clear_search") }}</button>
      <span class="cnt" aria-live="polite" aria-atomic="true">{{ t("ext.packages_shown_total", { shown: rows.length, total: packages.length }) }}</span>
    </div>
    <div v-if="packages.length > 0 && q && rows.length === 0" class="empty">{{ t("ext.no_matching_installed_packages") }}</div>

    <details v-for="p in rows" :key="p.name" class="srv" :data-st="p.enabled ? 'ready' : 'disabled'" :data-extension-name="p.name" :open="confirming === p.name || !!failedNote || undefined">
      <summary>
        <i class="pip" />
        <span class="srv-main">
          <span class="nm">{{ p.name }}</span>
          <span class="srv-sub">
            <span class="ds">{{ summary(p) || t("ext.nothing_usable_inside") }}</span>
            <span v-if="[p.version, p.manifestKind, p.source].filter(Boolean).length" class="ds">{{ [p.version, p.manifestKind, p.source].filter(Boolean).join(" · ") }}</span>
          </span>
        </span>
        <Switch
          data-action="extensions.enabled"
          :data-target="p.name"
          :on="p.enabled"
          :busy="busy === `toggle:${p.name}`"
          :label="`${t(p.enabled ? 'common.close' : 'ext.enable')} ${p.name}`"
          :on-click="() => void setEnabled(p)"
        />
      </summary>

      <div v-if="p.error" class="why">{{ p.error }}</div>
      <div v-if="p.status === 'disabled_incompatible'" class="why">{{ p.statusReason || t("ext.this_package_does_not_work") }}</div>
      <div v-for="w in p.warnings ?? []" :key="w" class="why">{{ w }}</div>

      <!-- What executes comes first: it is the part of a package a reader has
           to know about, and the part they cannot see from a name. -->
      <div class="peek">
        <div v-if="p.runtime" class="row" data-run>
          <span class="d">·</span>
          <span>{{ t("ext.long_running_process") }}</span>
          <span class="sc">{{ [p.runtime.command, ...(p.runtime.args ?? [])].join(" ") }}</span>
        </div>
        <div v-for="name in p.runtime?.tools ?? []" :key="`tool:${name}`" class="row" data-run>
          <span class="d">·</span>
          <span>{{ t("common.tool") }}</span>
          <span class="sc">{{ name }}</span>
        </div>
        <div v-for="(h, i) in p.hooks ?? []" :key="`hook:${i}`" class="row" data-run>
          <span class="d">·</span>
          <span>{{ h.event }}</span>
          <span class="sc">{{ h.description || h.command || h.contextFile }}</span>
        </div>
        <div v-for="s in p.mcpServers ?? []" :key="s.name" class="row" data-run>
          <span class="d">·</span>
          <span>{{ s.displayName || s.name }}</span>
          <span class="sc">{{ s.description || s.command || s.url || s.transport }}</span>
        </div>
        <div v-for="[label, items] in ([['ext.skills', p.skills], ['ext.commands', p.commands], ['ext.subagents', p.agents], ['ext.prompts', p.prompts], ['ext.themes', p.themes]] as [string, PluginItem[] | undefined][])" :key="label" class="contributions" :aria-label="t(label)">
          <template v-if="items?.length">
            <h4 class="kind">{{ t(label) }}</h4>
            <div v-for="(it, i) in items" :key="`${label}:${i}`" class="row">
              <span class="d">·</span>
              <span>{{ it.invocation || it.name }}</span>
              <span class="sc">{{ it.description }}</span>
              <CopyButton v-if="it.invocation" :text="it.invocation" icon-only :label="t('ext.copy_invocation_command', { command: it.invocation })" />
            </div>
          </template>
        </div>
        <div v-for="(s, i) in p.skipped ?? []" :key="`skip:${i}`" class="row">
          <span class="d">·</span>
          <span>{{ s.capability }}</span>
          <span class="sc">{{ t("ext.unusable_why", { why: s.reason }) }}</span>
        </div>
      </div>

      <span class="acts">
        <button v-if="p.source" class="act ghost" type="button" data-action="extensions.update" :data-target="p.name" :disabled="!!busy" @click="startUpdate(p)">
          {{ t("ext.update") }}
        </button>
        <button class="act ghost" type="button" data-action="extensions.export" :data-target="p.name" :disabled="!!busy" @click="void exportOne(p)">
          {{ t(busy === `export:${p.name}` ? "ext.packing" : "ext.export") }}
        </button>
        <button class="act ghost" type="button" :aria-label="t('common.remove_name', { name: p.name })" :disabled="!!busy" @click="confirming = p.name">
          {{ t("common.remove") }}
        </button>
      </span>

      <div v-if="exported && busy === '' && confirming === ''" class="why" role="status">
        {{ exported.savedTo ? t("ext.saved_to_path", { path: exported.savedTo }) : t("ext.exported") }}
        {{ " " }}{{ exported.required.length ? t("ext.the_key_values_were_stripped", { names: exported.required.join("、") }) : t("ext.this_package_asks_for_no") }}
      </div>

      <div v-if="confirming === p.name" class="confirm">
        <span class="q">{{ t("ext.delete_name_the_skills_commands", { name: p.name }) }}</span>
        <button class="act" type="button" :disabled="!!busy" @click="confirming = ''">{{ t("common.cancel") }}</button>
        <button class="act danger" type="button" data-action="extensions.remove" :data-target="p.name" :disabled="!!busy" @click="void removeOne(p)">
          {{ t(busy === `remove:${p.name}` ? "ext.removing_2" : "common.delete") }}
        </button>
      </div>
    </details>

    <div v-if="packages.length === 0 && !adding && !updating" class="empty">{{ t("settings.no_plugin_packages_installed") }}</div>

    <div v-if="!adding && !updating && !plan && !done" class="acts">
      <button class="act" type="button" data-action="extensions.add" @click="void startAdd('source')">{{ t("ext.add_from_source") }}</button>
      <button class="act" type="button" data-action="extensions.pick-folder" @click="void startAdd('folder')">{{ t("ext.add_from_folder") }}</button>
    </div>

    <!-- Confirmation: the plan as the kernel graded it, one row per thing that
         will run. A source that expands into several skills is up to the kernel;
         nothing here recounts what it will install. -->
    <div v-if="plan" class="addpkg" data-stage="confirm" :aria-busy="busy === 'install'">
      <div v-if="updating" class="find" :data-lvl="gained.length ? 'warn' : undefined">
        <span class="t">
          {{ version && updating.version ? `${updating.name} ${updating.version} → ${version}` : t("ext.reinstall_name", { name: updating.name }) }}
        </span>
        <span class="why">{{ gained.length ? t("ext.this_version_adds_list", { list: gained.join("、") }) : t("ext.nothing_new_that_runs") }}</span>
      </div>
      <section v-for="g in groups" :key="g.level" class="rgroup" :data-lvl="g.level">
        <h3>{{ t(KIND_TITLE[g.level] ?? g.level) }}</h3>
        <div v-for="(a, i) in g.actions" :key="`${a.kind}:${a.name}:${i}`" class="cand">
          <div class="cand-hd">
            <span class="nm">{{ a.name || a.kind }}</span>
            <span class="ds">{{ [a.version, a.manifestKind, a.transport].filter(Boolean).join(" · ") }}</span>
          </div>
          <div v-if="a.kind === 'plugin' && a.commit" class="risk">
            <span class="lb">{{ t("ext.resolved_commit") }}</span>
            <span class="dt">{{ a.commit }}</span>
            <CopyButton :text="a.commit" :label="t('ext.copy_commit')" icon-only show-feedback />
          </div>
          <div v-for="r in executes(a)" :key="r.label" class="risk" data-kind="shell">
            <span class="lb">{{ t(r.label) }}</span>
            <span class="dt">{{ r.detail }}</span>
            <span v-if="r.why" class="why">{{ t(r.why) }}</span>
          </div>
          <ModPreviewBlock v-if="a.mod" :mod="a.mod" />
          <div v-if="contributes(a).length > 0" class="risk">
            <span class="lb">{{ t("ext.will_be_added") }}</span>
            <span class="dt">{{ contributes(a).join(" · ") }}</span>
          </div>
          <div v-for="(_, k) in a.env ?? {}" :key="`env:${k}`" class="risk" data-kind="secret">
            <span class="lb">{{ t("ext.needs_filling_in") }}</span>
            <span class="dt">{{ k }}</span>
          </div>
          <div v-for="s in a.skippedCapabilities ?? []" :key="`${s.capability}:${s.reason}`" class="risk">
            <span class="lb">{{ t("ext.unusable") }}</span>
            <span class="dt">{{ s.capability }}</span>
            <span class="why">{{ s.reason }}</span>
          </div>
          <div v-if="a.previewTruncated" class="risk">
            <span class="lb">{{ t("ext.shortened") }}</span>
            <span class="dt">{{ t("ext.some_text") }}</span>
          </div>
          <!-- The kernel's own reasons are written for the model that reads the
               tool result; they stay readable behind a fold rather than driving
               the layout of a row. -->
          <details v-if="a.riskReasons?.length" class="reasons">
            <summary>{{ t("ext.the_kernels_assessment_n", { n: a.riskReasons.length }) }}</summary>
            <p v-for="r in a.riskReasons" :key="r">{{ r }}</p>
          </details>
        </div>
      </section>
      <div v-if="plan.hiddenActions || plan.previewTruncated" class="why">
        {{ plan.hiddenActions ? t("ext.n_more_steps_are_not", { n: plan.hiddenActions }) : "" }}{{ plan.hiddenActions && plan.previewTruncated ? " " : "" }}{{ plan.previewTruncated ? t("ext.some_text_is_too_long") : "" }}
      </div>
      <div v-for="w in plan.warnings ?? []" :key="w" class="why">{{ w }}</div>
      <div class="acts">
        <span class="note">{{ t(updating ? "ext.replace_the_installed_copy" : "ext.install_to_mine_available_in") }}</span>
        <button class="act" type="button" data-action="extensions.cancel" :disabled="busy === 'install'" @click="close">{{ t(updating ? "common.cancel" : "common.back") }}</button>
        <button class="act" type="button" :data-action="updating ? 'extensions.update' : 'extensions.install'" data-primary :disabled="busy === 'install'" @click="void install()">
          {{ t(busy === "install" ? (updating ? "ext.updating" : "ext.installing") : updating ? "ext.update" : "ext.install") }}
        </button>
      </div>
    </div>

    <!-- The plan landed but the rebuild could not: that is an outcome with a
         location, so it says both instead of reading as a failed install. -->
    <div v-if="done" class="addpkg" data-stage="done">
      <div class="find" :data-lvl="done.ok ? 'ok' : 'warn'" role="status">
        <span class="t">{{ done.ok ? t("ext.installed_but_the_runtime_did") : t("ext.not_all_items_were_installed") }}</span>
        <span v-if="done.reloadError" class="why">{{ done.reloadError }}</span>
        <span v-else-if="done.error" class="why">{{ done.error }}</span>
      </div>
      <div class="acts">
        <button v-if="!done.ok" class="act" type="button" data-primary @click="close">{{ t("common.retry") }}</button>
        <button class="act" type="button" data-action="extensions.finish" @click="close">{{ t("common.done") }}</button>
      </div>
    </div>

    <div v-if="adding === 'source' && !plan && !done" class="addpkg" data-stage="paste" :aria-busy="busy === 'look'">
      <textarea v-model="draftText" class="paste" rows="3" :aria-label="t('ext.add_from_source')" :placeholder="t('ext.a_repository_address_or_drop')" :disabled="busy === 'look'" @keydown.ctrl.enter.prevent="void look()" @keydown.meta.enter.prevent="void look()" />
      <div class="acts">
        <span class="note">{{ t("ext.add_from_source_hint") }}</span>
        <button class="act" type="button" data-action="extensions.cancel" @click="close">{{ t("common.cancel") }}</button>
        <button class="act" type="button" data-action="extensions.inspect" data-primary :disabled="!draftText.trim() || busy === 'look'" @click="void look()">
          {{ t(busy === "look" ? "ext.reading" : "ext.see_what_it_is") }}
        </button>
      </div>
    </div>

    <div v-if="updating && !plan && !done" class="addpkg" data-stage="reading" :aria-busy="busy === 'look'">
      <div class="find" :data-lvl="failedNote ? 'err' : undefined">
        <span class="t">{{ failedNote ? t("ext.cannot_read_name_s_source", { name: updating.name }) : t("ext.reading_name_s_source", { name: updating.name }) }}</span>
        <span class="why">{{ failedNote || updating.source }}</span>
      </div>
      <div class="acts">
        <button v-if="failedNote" class="act" type="button" data-action="extensions.inspect" data-primary @click="void look()">{{ t("common.retry") }}</button>
        <button class="act" type="button" data-action="extensions.cancel" @click="close">{{ t(failedNote ? "ext.close" : "common.cancel") }}</button>
      </div>
    </div>

    <div v-if="failedNote" class="why" role="alert">{{ failedNote }}</div>
  </div>
</template>
