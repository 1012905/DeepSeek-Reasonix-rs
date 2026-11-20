<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { HookCatalog, HookDryRun, HookEntry } from "../../../port/hook";
import Group from "../../Group.vue";
import Switch from "../../Switch.vue";

// Nobody arrives wanting a "PreToolUse hook with an anchored matcher". They
// arrive wanting the agent to run their formatter. The recipe is the product;
// the event, the matcher and the exit-code contract are its implementation.
interface Recipe {
  id: string;
  title: string;
  desc: string;
  event: string;
  match?: string;
  command: string;
}

const RECIPES: readonly Recipe[] = [
  {
    id: "format",
    title: "ext.format_files_after_they_are",
    desc: "ext.runs_a_formatter_after_every",
    event: "PostToolUse",
    match: "edit_file|write_file|multi_edit",
    command: "gofmt -w . 2>/dev/null || true",
  },
  {
    id: "guard-secrets",
    title: "ext.ask_me_before_writing_a",
    desc: "ext.stops_on_paths_like_env",
    event: "PreToolUse",
    match: "edit_file|write_file",
    command: `grep -q '"\\.env' <<< "$REASONIX_HOOK_PAYLOAD" && exit 2 || exit 0`,
  },
  {
    id: "test-before-stop",
    title: "ext.run_the_tests_before_wrapping",
    desc: "ext.runs_the_tests_at_the",
    event: "Stop",
    command: "go test ./... 2>&1 | tail -5",
  },
];

const SCOPE_LABEL: Record<Scope, string> = { user: "ext.mine", project: "ext.this_project" };
const SCOPE_NOTE: Record<Scope, string> = { user: "ext.this_machine_only", project: "ext.written_into_the_repository_so" };

const EVENT_LABELS: ReadonlyMap<string, string> = new Map<string, string>([
  ["PreToolUse", "ext.before_a_tool_runs"],
  ["PostToolUse", "ext.after_a_tool_runs"],
  ["PostToolUseFailure", "ext.after_a_tool_fails"],
  ["PermissionRequest", "ext.when_permission_is_requested"],
  ["UserPromptSubmit", "ext.when_a_message_is_submitted"],
  ["Stop", "ext.when_the_turn_ends"],
  ["StopFailure", "ext.when_the_turn_fails"],
  ["PostLLMCall", "ext.after_a_model_response_completes"],
  ["SessionStart", "ext.when_a_session_starts"],
  ["SessionEnd", "ext.when_a_session_ends"],
  ["SubagentStart", "ext.when_a_subagent_starts"],
  ["SubagentStop", "ext.when_a_subagent_ends"],
  ["Notification", "ext.when_your_attention_is_needed"],
  ["PreCompact", "ext.before_context_is_compacted"],
]);

const eventLabel = (event: string): string => {
  const key = EVENT_LABELS.get(event);
  return key ? `${t(key)} (${event})` : event;
};

type Scope = "user" | "project";
interface Trial { rule: HookEntry; result?: HookDryRun; error?: string }

// The section shell hands every section one prop; the block below is the
// session's own `hooks` anchor, rendered here as the whole of this section.
const props = defineProps<{ settingsProps: SettingsProps }>();
const port = computed(() => props.settingsProps.port);

const cat = ref<HookCatalog | null>(null);
const loaded = ref(false);
const scope = ref<Scope>("user");
const expert = ref(false);
const busy = ref("");
const error = ref("");
const tried = ref<Record<string, Trial>>({});
const draft = ref<HookEntry[]>([]);

// The kernel's answer about the event, not the client's guess: whether exit 2
// stops the agent is a property of the event.
const mine = computed(() => cat.value?.hooks.filter((h) => h.scope === (scope.value === "user" ? "global" : "project")) ?? []);
const plugin = computed(() => cat.value?.hooks.filter((h) => h.scope === "plugin") ?? []);
const broken = computed(() => cat.value?.sources.filter((s) => s.status === "malformed" || s.status === "unreadable") ?? []);
const ruleCount = computed(() => mine.value.length);

const hint = computed(() => {
  const head = t("settings.hooks_hint");
  return cat.value ? `${head} · ${t("settings.n_hooks", { n: cat.value.hooks.length })}` : head;
});

const blocking = (event: string) => cat.value?.events.find((e) => e.name === event)?.blocking ?? false;
const usesMatch = (event: string) => cat.value?.events.find((e) => e.name === event)?.usesMatch ?? false;

async function reload() {
  try {
    const next = await port.value.hooks();
    cat.value = next;
    // Project-scoped rules have nowhere to live without an open project.
    if (!next.projectPath && scope.value === "project") scope.value = "user";
  } catch {
    cat.value = null;
  } finally {
    loaded.value = true;
  }
}

watch(port, () => void reload(), { immediate: true });

// Re-seeded whenever the saved rules or the scope change, so a save never
// leaves a stale copy of the draft on screen.
watch([mine, scope], () => { draft.value = mine.value.map((h) => ({ ...h })); }, { immediate: true });

async function save(next: HookEntry[]) {
  busy.value = "save";
  error.value = "";
  try {
    await port.value.saveHooks(scope.value, next);
    await reload();
    props.settingsProps.onChanged();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = "";
  }
}

const has = (r: Recipe) => mine.value.some((h) => h.event === r.event && h.command === r.command);
const toggleRecipe = (r: Recipe) => void save(
  has(r)
    ? mine.value.filter((h) => !(h.event === r.event && h.command === r.command))
    : [...mine.value, { event: r.event, match: r.match, command: r.command, description: r.title, scope: scope.value }],
);

const trialKey = (at: number) => `${scope.value}-${at}`;
const trialOf = (h: HookEntry, at: number): Trial | undefined => {
  const trial = tried.value[trialKey(at)];
  return trial?.rule === h ? trial : undefined;
};

async function tryOne(h: HookEntry, at: number) {
  const key = trialKey(at);
  const trial: Trial = { rule: h };
  tried.value = { ...tried.value, [key]: trial };
  busy.value = key;
  error.value = "";
  try {
    const result = await port.value.dryRunHook(h);
    if (tried.value[key] === trial) tried.value = { ...tried.value, [key]: { ...trial, result } };
  } catch (e) {
    if (tried.value[key] === trial) tried.value = { ...tried.value, [key]: { ...trial, error: reason(e) } };
  } finally {
    busy.value = "";
  }
}

const patchRule = (at: number, p: Partial<HookEntry>) => {
  draft.value = draft.value.map((h, i) => (i === at ? { ...h, ...p } : h));
};
const onEvent = (at: number) => (e: Event) => patchRule(at, { event: (e.target as HTMLSelectElement).value });
const onMatch = (at: number) => (e: Event) => patchRule(at, { match: (e.target as HTMLInputElement).value });
const onCommand = (at: number) => (e: Event) => patchRule(at, { command: (e.target as HTMLInputElement).value });

const removeRule = (at: number) => void save(draft.value.filter((_, i) => i !== at));
const addRule = () => { draft.value = [...draft.value, { event: "PostToolUse", command: "", scope: scope.value }]; };
const saveDraft = () => void save(draft.value);

// The exit code is not the answer; what it does on this event is. blocks is the
// kernel's verdict for this event, so it is what the line reports.
const verdict = (res: HookDryRun): string =>
  res.timedOut ? t("ext.timed_out_this_step_counts") : res.blocks ? t("chat.blocked") : t("common.allow");
</script>

<template>
  <Group id="hooks" :title="t('settings.hooks')" :hint="hint">
    <div v-if="!cat" class="empty">{{ t(loaded ? "ext.could_not_read_the_hooks" : "common.loading") }}</div>
    <div v-else class="hooks">
      <div class="scope" role="radiogroup" :aria-label="t('ext.where_these_rules_live')">
        <button v-for="s in (['user', 'project'] as const)" :key="s" type="button" role="radio"
          data-action="hooks.scope" :data-value="s" :aria-checked="scope === s"
          :disabled="!!busy || (s === 'project' && !cat.projectPath)"
          @click="scope = s; error = ''">
          {{ t(SCOPE_LABEL[s]) }}
          <i>{{ t(SCOPE_NOTE[s]) }}</i>
        </button>
      </div>
      <p v-if="scope === 'user' || cat.projectPath" class="path">{{ scope === "user" ? cat.globalPath : cat.projectPath }}</p>
      <p v-else class="peernote">{{ t("ext.no_project_open") }}</p>

      <div v-for="s in broken" :key="s.path" class="why" role="alert">{{ `${s.path} ${t("ext.could_not_be_read")}${s.parseError || s.status}` }}</div>

      <div class="recipes">
        <div v-for="r in RECIPES" :key="r.id" class="recipe" :data-on="has(r) ? '' : undefined">
          <div class="tx">
            <span class="lb">{{ t(r.title) }}</span>
            <span class="ds">{{ t(r.desc) }}</span>
          </div>
          <Switch data-action="hooks.recipe" :data-target="r.id" :on="has(r)" :busy="!!busy" :label="t(r.title)" :on-click="() => toggleRecipe(r)" />
        </div>
      </div>

      <button class="more" type="button" data-action="hooks.expert" :aria-expanded="expert" @click="expert = !expert">
        {{ t(expert ? "common.collapse" : "ext.write_one_manually") }}
        <span class="c">{{ t("common.n_rules", { n: ruleCount }) }}</span>
      </button>

      <div v-if="expert" class="expert">
        <div v-for="(h, i) in draft" :key="i" class="rule" :data-blocking="blocking(h.event) ? '' : undefined">
          <div class="line">
            <select :value="h.event" :aria-label="t('ext.events_it_hooks')" :disabled="!!busy" @change="onEvent(i)">
              <option v-for="e in cat.events" :key="e.name" :value="e.name">{{ eventLabel(e.name) }}</option>
            </select>
            <input v-if="usesMatch(h.event)" class="match" :value="h.match ?? ''" :placeholder="t('ext.which_tools_to_match_regex')"
              :aria-label="t('ext.which_tools_to_match_regex')" :disabled="!!busy" @input="onMatch(i)" />
            <i v-if="blocking(h.event)" class="warn">{{ t("ext.can_stop_the_agent") }}</i>
            <button class="act ghost" type="button" data-action="hooks.remove" :disabled="busy === 'save'" @click="removeRule(i)">{{ t("common.delete") }}</button>
          </div>
          <input class="cmd" :value="h.command" :placeholder="t('ext.the_command_to_run')" :aria-label="t('ext.the_command_to_run')"
            :disabled="!!busy" @input="onCommand(i)" />
          <div v-for="msg in h.issues ?? []" :key="msg" class="why">{{ msg }}</div>
          <div class="line">
            <button class="act" type="button" data-action="hooks.run-once" :disabled="!!busy" @click="tryOne(h, i)">
              {{ t(busy === trialKey(i) ? "ext.running" : "ext.dry_run_once") }}
            </button>
            <!-- The command really runs. Saying so is the difference between a
                 button that checks syntax and one that might delete a file. -->
            <span class="note">{{ t("ext.really_runs_this_command") }}</span>
          </div>
          <span v-if="trialOf(h, i)?.result" class="dryrun" :data-blocks="trialOf(h, i)?.result?.blocks ? '' : undefined">
            <b>{{ verdict(trialOf(h, i)!.result!) }}</b>
            <span class="peernote">{{ `${t("panels.exit_code", { code: trialOf(h, i)?.result?.exitCode ?? 0 })} · ${trialOf(h, i)?.result?.durationMs ?? 0} ${t("ext.ms")}` }}</span>
            <span v-if="trialOf(h, i)?.result?.stdout || trialOf(h, i)?.result?.stderr" class="out">{{
              (trialOf(h, i)?.result?.stdout || trialOf(h, i)?.result?.stderr || "").slice(0, 160)
            }}</span>
          </span>
          <div v-if="trialOf(h, i)?.error" class="why">{{ trialOf(h, i)?.error }}</div>
        </div>

        <div class="line">
          <button class="act" type="button" data-action="hooks.add" :disabled="busy === 'save'" @click="addRule">{{ t("common.add_a_rule") }}</button>
          <button class="act" type="button" data-action="hooks.save" data-primary="true" :disabled="busy === 'save'" @click="saveDraft">
            {{ t(busy === "save" ? "common.saving" : "common.save") }}
          </button>
        </div>

        <div v-if="plugin.length > 0" class="fromplugin">
          <span class="lb">{{ t("ext.n_from_plugins", { n: plugin.length }) }}</span>
          <div v-for="(h, i) in plugin" :key="i" class="hookrow">
            <span class="ev" :title="eventLabel(h.event)">{{ eventLabel(h.event) }}</span>
            <span class="cm">{{ h.command }}</span>
            <span class="peernote">{{ t("common.read_only") }}</span>
          </div>
        </div>
      </div>

      <div v-if="error" class="why" role="alert">{{ error }}</div>
    </div>
  </Group>
</template>
