<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason, savedNotApplied } from "../../../i18n/kernel";
import type { AgentPort, PermissionLists, PermissionRules } from "../../../port/port";
import Switch from "../../Switch.vue";
import SaveNote from "../../SaveNote.vue";
import { SegChoice } from "../../SettingRow.vue";
import type { SaveNoteState } from "../../SaveNote.vue";

type List = "deny" | "ask" | "allow";

// The three verdicts in the order the gate reads them: a rule's verdict is a
// property of the rule, so moving one between lists is a change of what
// happens, not a delete-and-retype.
const LEVELS: readonly (readonly [List, string])[] = [
  ["deny", "common.refuse"],
  ["ask", "safety.ask"],
  ["allow", "common.allow"],
];

const MODES: readonly (readonly [string, string, string])[] = [
  ["ask", "safety.ask_for_confirmation", "safety.a_write_no_rule_matched"],
  ["allow", "common.allow", "safety.a_write_matched_by_no"],
  ["deny", "common.refuse", "safety.a_write_no_list_matched"],
];

// The tools worth offering by name. `file_mutation` is not a tool but the gate's
// alias for every writing tool, which is the one people usually mean.
const TOOLS: readonly (readonly [string, string])[] = [
  ["bash", "chat.commands"],
  ["file_mutation", "safety.writing_files_every_write_tool"],
  ["read_file", "safety.reading_files"],
  ["write_file", "safety.whole_file_writes"],
  ["edit_file", "safety.file_edits"],
  ["web_fetch", "safety.fetching_web_pages"],
  ["grep", "safety.full_text_search"],
  ["glob", "safety.finding_files"],
];

const TOOL_LABEL = new Map(TOOLS);

interface Recipe { id: string; title: string; desc: string; list: List; rules: string[] }

const RECIPES: readonly Recipe[] = [
  { id: "secrets", title: "safety.keep_env_off_limits", desc: "safety.file_tools_are_refused_outright", list: "deny", rules: ["file_mutation(*.env*)", "read_file(*.env*)"] },
  { id: "push", title: "safety.never_push", desc: "safety.change_anything_locally_pushing_stays", list: "deny", rules: ["bash(git push:*)"] },
  { id: "history", title: "safety.never_rewrite_git_history", desc: "safety.rebase_and_reset_can_eat", list: "deny", rules: ["bash(git rebase:*)", "bash(git reset:*)"] },
  { id: "tests", title: "safety.tests_without_asking", desc: "safety.test_commands_go_straight_through", list: "allow", rules: ["bash(go test:*)", "bash(npm test:*)", "bash(pytest:*)"] },
];

// A rule is "tool" or "tool(subject)"; splitting it lets the tool be said once
// per group instead of once per row.
function splitRule(rule: string): { tool: string; pattern: string } {
  const at = rule.indexOf("(");
  if (at < 0 || !rule.endsWith(")")) return { tool: rule.trim(), pattern: "" };
  return { tool: rule.slice(0, at).trim(), pattern: rule.slice(at + 1, -1) };
}

const joinRule = (tool: string, pattern: string) => (pattern.trim() ? `${tool.trim()}(${pattern.trim()})` : tool.trim());

// What a rule is compared against differs per tool, and the same glob means
// different things in each, so the group header says which.
const MATCHING: Record<string, string> = {
  bash: "safety.matched_by_command_words",
  grep: "safety.matched_against_the_search_pattern",
  glob: "safety.matched_against_the_search_pattern",
  web_fetch: "safety.matched_against_the_url",
};
const PATH_MATCHING = "safety.matched_by_path_crosses";

// What is unusual about this one rule, and nothing where it is the common
// shape: printing "commands starting with x" on most rows is noise.
function odd(tool: string, pattern: string): string {
  const p = pattern.trim();
  if (!p) return t("safety.every_call_to_this_tool");
  if (tool === "bash") {
    if (p.endsWith(":*")) return "";
    if (p.includes("*")) return t("safety.matches_the_whole_command_not");
    return t("safety.exactly_this_command_one_more");
  }
  if (p.includes("*")) return "";
  return MATCHING[tool] ? t("safety.exactly_this_one_value") : t("safety.exactly_this_one_path");
}

// The add form is the one place the full sentence belongs: nothing is on screen
// yet to compare the rule against, and it has not been saved.
function explain(tool: string, pattern: string): string {
  const p = pattern.trim();
  if (!p) return t("safety.every_call_to_this_tool");
  if (tool === "bash") {
    if (p.endsWith(":*")) return t("safety.commands_starting_with_cmd_whatever", { cmd: p.slice(0, -2) });
    if (p.includes("*")) return t("safety.commands_matching_pat_as_a", { pat: p });
    return t("safety.exactly_the_command_cmd", { cmd: p });
  }
  if (p.includes("*")) return t("safety.calls_whose_path_matches_pat", { pat: p });
  return t("safety.exactly_the_path_path", { path: p });
}

interface Row { rule: string; pattern: string; level: List }
interface Group { tool: string; rows: Row[] }

const ORDER: Record<List, number> = { deny: 0, ask: 1, allow: 2 };

function group(lists: PermissionLists, query: string): Group[] {
  const q = query.trim().toLocaleLowerCase();
  const byTool = new Map<string, Group>();
  for (const level of ["deny", "ask", "allow"] as List[]) {
    for (const rule of lists[level]) {
      const { tool, pattern } = splitRule(rule);
      if (q && !rule.toLocaleLowerCase().includes(q)) continue;
      let g = byTool.get(tool);
      if (!g) {
        g = { tool, rows: [] };
        byTool.set(tool, g);
      }
      g.rows.push({ rule, pattern, level });
    }
  }
  for (const g of byTool.values()) g.rows.sort((a, b) => ORDER[a.level] - ORDER[b.level] || a.pattern.localeCompare(b.pattern));
  // Most-governed tool first: that is the one being read.
  return [...byTool.values()].sort((a, b) => b.rows.length - a.rows.length || a.tool.localeCompare(b.tool));
}

const props = defineProps<{ port: AgentPort; onChanged: () => void }>();

const rules = ref<PermissionRules | null>(null);
const busy = ref("");
const note = ref<SaveNoteState | null>(null);
const query = ref("");
const adding = ref(false);
const shut = ref(new Set<string>());

// The add form's own draft, kept apart so a save that reloads the lists does
// not wipe what is half typed.
const draftTool = ref("bash");
const draftPattern = ref("");
const draftLevel = ref<List>("deny");

watch(() => props.port, (port) => {
  let live = true;
  rules.value = null;
  note.value = null;
  port.permissions()
    .then((value) => { if (live) rules.value = value; })
    .catch(() => { if (live) rules.value = null; });
  return () => { live = false; };
}, { immediate: true });

const lists = computed<PermissionLists | null>(() => {
  const r = rules.value;
  return r ? { mode: r.mode, deny: r.deny, ask: r.ask, allow: r.allow } : null;
});
const groups = computed(() => (lists.value ? group(lists.value, query.value) : []));
const counts = computed(() => ({
  deny: rules.value?.deny.length ?? 0,
  ask: rules.value?.ask.length ?? 0,
  allow: rules.value?.allow.length ?? 0,
}));
const total = computed(() => counts.value.deny + counts.value.ask + counts.value.allow);
const dormant = computed(() => new Set((rules.value?.dormant ?? []).map((d) => `${d.list}\u0000${d.rule}`)));
const draftRule = computed(() => joinRule(draftTool.value, draftPattern.value));

const modeChoice = computed(() => SegChoice<string>({
  label: t("safety.everything_else_that_writes"),
  value: rules.value?.mode ?? "ask",
  options: MODES.map(([id, name]) => [id, t(name)] as const),
  disabled: !!busy.value,
  action: "permissions.mode",
  onPick: (next) => void apply("mode", { ...(lists.value as PermissionLists), mode: next }),
}));

const apply = async (what: string, next: PermissionLists) => {
  busy.value = what;
  note.value = null;
  try {
    rules.value = await props.port.savePermissions(next);
    props.onChanged();
  } catch (e) {
    note.value = { text: reason(e), unapplied: savedNotApplied(e) };
  } finally {
    busy.value = "";
  }
};

const revoke = async (rule: string) => {
  busy.value = rule || "all";
  note.value = null;
  try {
    rules.value = await props.port.revokeSessionGrant(rule);
  } catch (e) {
    note.value = { text: reason(e), unapplied: savedNotApplied(e) };
  } finally {
    busy.value = "";
  }
};

const revokeRemembered = async (rule: string) => {
  busy.value = `remembered:${rule}`;
  note.value = null;
  try {
    rules.value = await props.port.revokeRememberedProjectRule(rule);
    props.onChanged();
  } catch (e) {
    note.value = { text: reason(e), unapplied: savedNotApplied(e) };
  } finally {
    busy.value = "";
  }
};

const without = (l: PermissionLists, rule: string): PermissionLists => ({
  ...l,
  deny: l.deny.filter((r) => r !== rule),
  ask: l.ask.filter((r) => r !== rule),
  allow: l.allow.filter((r) => r !== rule),
});

// Changing a verdict is a move between lists rather than a delete and a retype.
const move = (rule: string, to: List) => {
  const now = lists.value;
  if (!now) return;
  const next = without(now, rule);
  void apply(rule, { ...next, [to]: [...next[to], rule] });
};

const drop = (rule: string) => {
  if (!lists.value) return;
  void apply(rule, without(lists.value, rule));
};

const add = (rule: string, level: List) => {
  const now = lists.value;
  if (!now || now[level].includes(rule)) return;
  const next = without(now, rule);
  void apply(rule, { ...next, [level]: [...next[level], rule] });
};

const addDraft = () => {
  if (!draftRule.value) return;
  add(draftRule.value, draftLevel.value);
  adding.value = false;
  draftPattern.value = "";
};

const has = (recipe: Recipe) => !!lists.value && recipe.rules.every((rule) => lists.value![recipe.list].includes(rule));

const toggleRecipe = (recipe: Recipe) => {
  const now = lists.value;
  if (!now) return;
  const on = has(recipe);
  let next = now;
  for (const rule of recipe.rules) next = without(next, rule);
  if (!on) next = { ...next, [recipe.list]: [...next[recipe.list], ...recipe.rules] };
  void apply(recipe.id, next);
};

const toggleGroup = (tool: string) => {
  const next = new Set(shut.value);
  if (next.has(tool)) next.delete(tool);
  else next.add(tool);
  shut.value = next;
};

// Whether anything in the group is a rule the gate can never match.
const allDormant = (g: Group) => g.rows.every((row) => dormant.value.has(`${row.level}\u0000${row.rule}`));
const modeNote = computed(() => t(MODES.find(([id]) => id === rules.value?.mode)?.[2] ?? ""));
</script>

<template>
  <div v-if="!rules || !lists" class="empty">{{ t("safety.cannot_read_the_permission_config") }}</div>
  <div v-else class="rules">
    <div v-if="rules.shadowedBy" class="find" data-lvl="warn" role="status">
      <span class="t">{{ t("safety.this_project_ships_its_own") }}</span>
      <span class="why">{{ t("safety.path_declares_permissions_too_and", { path: rules.shadowedBy }) }}</span>
    </div>

    <div v-if="rules.dormant?.length" class="find" data-lvl="warn" role="status">
      <span class="t">{{ t("safety.n_rule_s_name_no", { n: rules.dormant.length }) }}</span>
      <span class="why">{{ t("safety.a_shell_command_is_written") }}</span>
    </div>

    <div class="recipes">
      <div v-for="r in RECIPES" :key="r.id" class="recipe" :data-on="has(r) ? '' : undefined">
        <div class="tx">
          <span class="lb">{{ t(r.title) }}</span>
          <span class="ds">{{ t(r.desc) }}</span>
        </div>
        <Switch
          data-action="permissions.recipe"
          :data-target="r.id"
          :on="has(r)"
          :busy="busy === r.id"
          :label="t(r.title)"
          :on-click="() => toggleRecipe(r)"
        />
      </div>
    </div>

    <div class="fallback">
      <span class="k">{{ t("safety.everything_else_that_writes") }}</span>
      <component :is="modeChoice" />
    </div>
    <p class="note">{{ modeNote }}</p>

    <div v-if="rules.rememberedErrorCode" class="find" data-lvl="warn" role="status">
      <span class="t">{{ t("safety.could_not_read_remembered_permission") }}</span>
      <span class="why">{{ t("safety.check_or_repair_the_grant") }}</span>
      <span class="path">{{ rules.rememberedPath }}</span>
    </div>

    <section v-if="rules.remembered?.length" class="rtable" :aria-label="t('safety.remembered_allow_rules_for_this')">
      <div class="rhead">{{ t("safety.remembered_allow_rules_for_this") }}</div>
      <div v-for="rule in rules.remembered" :key="rule" class="rrow" data-k="allow">
        <i class="dot" aria-hidden="true" />
        <span class="pat"><code>{{ rule }}</code></span>
        <button
          class="act ghost"
          type="button"
          data-action="permissions.revoke-remembered"
          :data-target="rule"
          :disabled="!!busy"
          :aria-label="t('safety.take_back_rule', { rule })"
          @click="void revokeRemembered(rule)"
        >{{ t("safety.take_back") }}</button>
      </div>
      <p class="path">{{ rules.rememberedPath }}</p>
    </section>

    <!-- One header for the whole boundary: how big it is, how to find a row in
         it, and the single way in. Three add boxes made one act look like three. -->
    <div class="rhead">
      <span class="n">
        {{ total ? t("common.n_rules", { n: total }) : t("safety.no_rules_of_your_own") }}
        <span v-if="total > 0" class="mix">
          <i data-k="deny" :title="t('common.refuse')">{{ counts.deny }}</i>
          <i data-k="ask" :title="t('safety.ask')">{{ counts.ask }}</i>
          <i data-k="allow" :title="t('common.allow')">{{ counts.allow }}</i>
        </span>
      </span>
      <input v-if="total > 6" v-model="query" class="find-rule" :placeholder="t('safety.filter')" :aria-label="t('safety.filter')" />
      <button class="act" type="button" data-action="permissions.add-rule" @click="adding = !adding">
        {{ t(adding ? "common.cancel" : "common.add_a_rule") }}
      </button>
    </div>

    <div v-if="adding" class="raddbox">
      <div class="line">
        <select v-model="draftTool" :aria-label="t('common.tool')" :disabled="!!busy">
          <option v-for="[id, label] in TOOLS" :key="id" :value="id">{{ `${id} · ${t(label)}` }}</option>
        </select>
        <input
          v-model="draftPattern"
          :placeholder="t(draftTool === 'bash' ? 'safety.e_g_git_push_empty' : 'safety.e_g_env_empty_the')"
          :disabled="!!busy"
          @keydown.enter="addDraft"
        />
        <select v-model="draftLevel" :aria-label="t('safety.verdict')" :disabled="!!busy">
          <option v-for="[id, label] in LEVELS" :key="id" :value="id">{{ t(label) }}</option>
        </select>
        <button class="act" type="button" data-primary :disabled="!!busy || !draftRule" @click="addDraft">{{ t("common.add") }}</button>
      </div>
      <p class="preview">
        <code>{{ draftRule || "…" }}</code>
        <span>{{ t(LEVELS.find(([id]) => id === draftLevel)?.[1] ?? "") }}</span>
        <span class="says">{{ explain(draftTool, draftPattern) }}</span>
      </p>
    </div>

    <!-- What a prompt allowed for this session is in no file, so a reader of the
         table above is reading less than the agent may currently do. -->
    <section v-if="rules.granted?.length" class="granted" :aria-label="t('safety.n_more_allowed_for_this', { n: rules.granted.length })">
      <div class="g-hd">
        <span class="n">{{ t("safety.n_more_allowed_for_this", { n: rules.granted.length }) }}</span>
        <span class="how">{{ t("safety.this_session_only_not_written") }}</span>
        <button class="act" type="button" data-action="permissions.revoke-session" data-value="all" :disabled="!!busy" @click="void revoke('')">
          {{ t("safety.take_all_back") }}
        </button>
      </div>
      <div v-for="rule in rules.granted" :key="rule" class="g-row">
        <code>{{ rule }}</code>
        <button
          class="act"
          type="button"
          data-action="permissions.revoke-session"
          :data-target="rule"
          :disabled="!!busy"
          :aria-label="t('safety.take_back_rule', { rule })"
          @click="void revoke(rule)"
        >{{ t("safety.take_back") }}</button>
      </div>
    </section>

    <div class="rtable">
      <section v-for="g in groups" :key="g.tool" class="rgroup">
        <button class="rg-hd" type="button" :aria-expanded="!shut.has(g.tool)" @click="toggleGroup(g.tool)">
          <i class="caret" aria-hidden="true" />
          <code>{{ g.tool }}</code>
          <span class="what">{{ TOOL_LABEL.has(g.tool) ? t(TOOL_LABEL.get(g.tool)!) : "" }}</span>
          <span v-if="!allDormant(g)" class="how">{{ t(MATCHING[g.tool] ?? PATH_MATCHING) }}</span>
          <span class="cnt">{{ t("common.n_2", { n: g.rows.length }) }}</span>
        </button>
        <template v-if="!shut.has(g.tool)">
          <div v-for="row in g.rows" :key="row.rule" class="rrow" :data-k="row.level">
            <i class="dot" aria-hidden="true" />
            <span class="pat">
              <code>{{ row.pattern || t("safety.the_whole_tool") }}</code>
              <span v-if="dormant.has(`${row.level}\u0000${row.rule}`)" class="says" data-dormant="">
                {{ t("safety.no_tool_is_named_tool", { tool: g.tool }) }}
              </span>
              <span v-else-if="odd(g.tool, row.pattern)" class="says">{{ odd(g.tool, row.pattern) }}</span>
            </span>
            <select
              :value="row.level"
              data-action="permissions.rule-level"
              :data-target="row.rule"
              :disabled="!!busy"
              :aria-label="t('safety.what_happens_on_rule', { rule: row.rule })"
              @change="move(row.rule, ($event.target as HTMLSelectElement).value as List)"
            >
              <option v-for="[id, label] in LEVELS" :key="id" :value="id">{{ t(label) }}</option>
            </select>
            <button
              class="act ghost"
              type="button"
              data-action="permissions.remove-rule"
              :data-target="row.rule"
              :disabled="!!busy"
              :aria-label="t('safety.delete_rule', { rule: row.rule })"
              @click="drop(row.rule)"
            >{{ t("common.delete") }}</button>
          </div>
        </template>
      </section>
      <div v-if="total > 0 && groups.length === 0" class="empty">{{ t("safety.no_rule_matches_that") }}</div>
    </div>

    <p v-if="total > 0" class="path">{{ rules.path }}</p>
    <SaveNote :note="note" />
  </div>
</template>
