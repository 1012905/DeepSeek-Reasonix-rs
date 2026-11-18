<script setup lang="ts">
import { computed, ref, shallowRef, watch, type Ref } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import type {
  AgentPort, AskTimeoutSettings, ProgressWatchSettings, SessionRefSettings, TurnNarrationSettings,
} from "../../../port/port";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import SectionCard from "../../SectionCard.vue";
import SettingRow from "../../SettingRow.vue";
import Switch from "../../Switch.vue";
import AgentScheduledRuns from "../parts/agent-scheduled-runs.vue";
import Hooks from "./hooks.vue";
import PersonaSection from "./persona.vue";
import PeersSection from "./peers.vue";
import { declarePage } from "./nesting";

// Everything on this page is one value in the user file, read back after every
// write: the kernel's answer is what the row shows, never the draft.
//
// Persona and peers fold into this page (FOLDS_INTO) and each has a file of its
// own, because a link — or a setting the model names — opens the sheet at
// "persona:agent-presets" or "peers:peers" directly. Their blocks are drawn
// here from those files rather than copied: the index under this page's title
// promises their anchors, and one implementation is what keeps the two
// spellings from drifting. The column is only ever one page, so `at` being
// "persona" mounts persona.vue and never this file — see ./nesting for the
// guard that keeps it that way if a page ever does nest what it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
declarePage("session");
const port = computed(() => props.settingsProps.port);
const announce = () => props.settingsProps.onChanged();

/** Reads a block's state once per port and reports a refused write. */
function useBlock<T>(load: (p: AgentPort) => Promise<T>) {
  const state = shallowRef(null) as Ref<T | null>;
  const busy = ref(false);
  const error = ref("");
  watch(port, (p, _was, onCleanup) => {
    let live = true;
    load(p).then((s) => { if (live) state.value = s; }, () => { if (live) state.value = null; });
    onCleanup(() => { live = false; });
  }, { immediate: true });
  const write = async (work: (p: AgentPort) => Promise<T>) => {
    busy.value = true;
    error.value = "";
    try {
      state.value = await work(port.value);
      announce();
    } catch (e) {
      error.value = reason(e);
    } finally {
      busy.value = false;
    }
  };
  return { state, busy, error, write };
}

const digits = (v: string) => v.replace(/\D/g, "");
const whole = (text: string, min: number, max: number): number | null => {
  const n = Number(text);
  return Number.isInteger(n) && n >= min && n <= max ? n : null;
};
// Enter commits by leaving the field, so one keystroke reaches the kernel once.
const blurOnEnter = (e: KeyboardEvent) => { if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur(); };
const textOf = (e: Event) => (e.target as HTMLInputElement).value;

// Two-way choices are drawn as one segmented control each: the id is what the
// action carries, the key is what the reader sees.
type Option<V extends string> = { id: V; label: string };
const PLAN_OPTIONS: readonly Option<Approval>[] = [
  { id: "ask", label: "settings.every_time" },
  { id: "auto", label: "settings.not_needed" },
];
const NARRATION_OPTIONS: readonly Option<Level>[] = [
  { id: "brief", label: "settings.brief" },
  { id: "detailed", label: "settings.detailed" },
];
const CLAIM_OPTIONS: readonly Option<Claims>[] = [
  { id: "task", label: "settings.per_task" },
  { id: "all", label: "settings.whole_workspace" },
];

// --- ask-timeout -------------------------------------------------------------

const ask = useBlock((p) => p.askTimeout());
const askDraft = ref("");
watch(ask.state, (s: AskTimeoutSettings | null) => { if (s) askDraft.value = String(s.seconds || s.defaultSeconds); }, { immediate: true });
const askOn = computed(() => (ask.state.value?.seconds ?? 0) > 0);
const askMin = computed(() => ask.state.value?.minSeconds ?? 0);
const askMax = computed(() => ask.state.value?.maxSeconds ?? 0);
const askInvalid = computed(() => ask.state.value?.invalid ?? "");
const askPath = computed(() => ask.state.value?.path ?? "");

const toggleAsk = () => {
  const s = ask.state.value;
  if (s) void ask.write((p) => p.saveAskTimeout(s.seconds > 0 ? 0 : s.defaultSeconds));
};
const onAskInput = (e: Event) => { askDraft.value = digits(textOf(e)); };
const commitAsk = () => {
  const s = ask.state.value;
  if (!s || s.seconds === 0) return;
  const n = whole(askDraft.value, s.minSeconds, s.maxSeconds);
  if (n === null) {
    ask.error.value = t("common.enter_a_whole_number_from", { min: s.minSeconds, max: s.maxSeconds });
    askDraft.value = String(s.seconds);
    return;
  }
  if (n !== s.seconds) void ask.write((p) => p.saveAskTimeout(n));
};

// --- turn-narration ----------------------------------------------------------

type Level = TurnNarrationSettings["level"];
const narration = useBlock((p) => p.turnNarrationSettings());
const narrationLevel = computed<Level>(() => narration.state.value?.level ?? "brief");
const pickNarration = (next: Level) => {
  if (narration.state.value?.level === next) return;
  void narration.write((p) => p.saveTurnNarrationSettings(next));
};

// --- progress-watch ----------------------------------------------------------

const LIMIT = 1000;
const progress = useBlock((p) => p.progressWatch());
const rounds = ref("");
const multiple = ref("");
watch(progress.state, (s: ProgressWatchSettings | null) => {
  if (!s) return;
  rounds.value = String(s.rounds);
  multiple.value = String(s.tokenMultiple);
}, { immediate: true });

const paused = computed(() => progress.state.value?.pause ?? false);
const defaultRounds = computed(() => progress.state.value?.defaultRounds ?? 0);
const defaultMultiple = computed(() => progress.state.value?.defaultTokenMultiple ?? 0);

const patchProgress = (s: ProgressWatchSettings, next: Partial<Pick<ProgressWatchSettings, "pause" | "rounds" | "tokenMultiple">>) =>
  void progress.write((p) => p.saveProgressWatch({ pause: s.pause, rounds: s.rounds, tokenMultiple: s.tokenMultiple, ...next }));
const togglePause = () => {
  const s = progress.state.value;
  if (s) patchProgress(s, { pause: !s.pause });
};
const onRoundsInput = (e: Event) => { rounds.value = digits(textOf(e)); };
const onMultipleInput = (e: Event) => { multiple.value = digits(textOf(e)); };
const commitRounds = () => {
  const s = progress.state.value;
  if (!s) return;
  const n = whole(rounds.value, 1, LIMIT);
  if (n === null) {
    progress.error.value = t("common.enter_a_whole_number_from_2", { n: LIMIT });
    rounds.value = String(s.rounds);
    return;
  }
  if (n !== s.rounds) patchProgress(s, { rounds: n });
};
const commitMultiple = () => {
  const s = progress.state.value;
  if (!s) return;
  const n = whole(multiple.value, 1, LIMIT);
  if (n === null) {
    progress.error.value = t("common.enter_a_whole_number_from_2", { n: LIMIT });
    multiple.value = String(s.tokenMultiple);
    return;
  }
  if (n !== s.tokenMultiple) patchProgress(s, { tokenMultiple: n });
};

// --- plan-approval -----------------------------------------------------------

type Approval = "ask" | "auto";
const plan = useBlock((p) => p.planApproval());
const planValue = computed<Approval>(() => (plan.state.value?.auto ? "auto" : "ask"));
const pickPlan = (next: Approval) => {
  if (planValue.value === next || !plan.state.value) return;
  void plan.write((p) => p.savePlanApproval(next === "auto"));
};

// --- session-ref -------------------------------------------------------------

const refs = useBlock((p) => p.sessionRefSettings());
const refDraft = ref("");
watch(refs.state, (s: SessionRefSettings | null) => { if (s) refDraft.value = String(s.tokens); }, { immediate: true });
const refAtDefault = computed(() => {
  const s = refs.state.value;
  return !!s && s.tokens === s.defaultTokens;
});
const onRefInput = (e: Event) => { refDraft.value = digits(textOf(e)); };
const commitRef = () => {
  const s = refs.state.value;
  if (!s) return;
  const n = whole(refDraft.value, s.minTokens, s.maxTokens);
  if (n === null) {
    refs.error.value = t("common.enter_a_whole_number_from", { min: s.minTokens, max: s.maxTokens });
    refDraft.value = String(s.tokens);
    return;
  }
  if (n !== s.tokens) void refs.write((p) => p.saveSessionRefSettings(n));
};
const resetRef = () => void refs.write((p) => p.saveSessionRefSettings(0));

// --- subagent-claims ---------------------------------------------------------

type Claims = "task" | "all";
const claims = useBlock((p) => p.subagentClaims());
const claimsValue = computed<Claims>(() => (claims.state.value?.optimistic ? "task" : "all"));
const pickClaims = (next: Claims) => {
  if (claimsValue.value === next || !claims.state.value) return;
  void claims.write((p) => p.saveSubagentClaims(next === "task"));
};
</script>

<template>
  <SectionCard :title="t('settings.plans_and_questions')">
    <SettingRow id="plan-approval" :title="t('settings.plan_approval')" :desc="t('settings.show_plans_before_running')"
      :more="t('plan.a_submitted_plan_starts_without')">
      <div v-if="!plan.state.value" class="empty">{{ t("plan.could_not_read_the_plan") }}</div>
      <div v-else class="seg" data-text="true" role="radiogroup" :aria-label="t('settings.plan_approval')">
        <button v-for="opt in PLAN_OPTIONS" :key="opt.id" type="button" role="radio" data-action="plan-approval.auto"
          :data-value="opt.id" :aria-checked="planValue === opt.id" :disabled="plan.busy.value" @click="pickPlan(opt.id)">
          {{ t(opt.label) }}
        </button>
      </div>
    </SettingRow>
    <div v-if="plan.error.value" class="why" role="alert">{{ plan.error.value }}</div>

    <SettingRow id="ask-timeout" :title="t('safety.question_wait_time')" :desc="t('settings.skip_after_no_answer')">
      <div v-if="!ask.state.value" class="empty">{{ t("safety.the_question_wait_setting_could") }}</div>
      <template v-else>
        <span v-if="askOn" class="unit-field">
          <input type="text" inputmode="numeric" :aria-label="t('safety.wait_time')" data-action-keydown="ask-timeout.seconds"
            data-action-change="ask-timeout.seconds" :value="askDraft" :disabled="ask.busy.value"
            @input="onAskInput" @blur="commitAsk" @keydown="blurOnEnter" />
          <i>{{ t("common.s") }}</i>
        </span>
        <Switch data-action="ask-timeout.enabled" :on="askOn" :busy="ask.busy.value"
          :label="t('safety.skip_a_question_nobody_answers')" :on-click="toggleAsk" />
      </template>
    </SettingRow>
    <div v-if="askInvalid" class="why">{{ t("safety.the_question_wait_in_the", { min: askMin, max: askMax, path: askPath }) }}</div>
    <div v-if="ask.error.value" class="why" role="alert">{{ ask.error.value }}</div>

    <SettingRow id="turn-narration" :title="t('settings.narration_between_tool_calls')" :desc="t('settings.say_what_a_step_did')">
      <div class="seg" data-text="true" role="radiogroup" :aria-label="t('settings.narration_between_tool_calls')">
        <button v-for="opt in NARRATION_OPTIONS" :key="opt.id" type="button" role="radio" data-action="chrome.turn-narration"
          :data-value="opt.id" :aria-checked="narrationLevel === opt.id" :disabled="narration.state.value === null"
          @click="pickNarration(opt.id)">
          {{ t(opt.label) }}
        </button>
      </div>
    </SettingRow>
    <div v-if="narration.error.value" class="why" role="alert">{{ narration.error.value }}</div>

    <SettingRow id="progress-watch" :title="t('panels.long_stretches_without_progress')" :desc="t('settings.pause_when_stuck')">
      <div v-if="!progress.state.value" class="empty">{{ t("panels.could_not_read_the_progress") }}</div>
      <Switch v-else data-action="progress-watch.pause" :on="paused" :busy="progress.busy.value"
        :label="t('panels.pause_a_task_that_stops')" :on-click="togglePause" />
    </SettingRow>
    <div v-if="progress.state.value" class="srow-more">
      <div class="lrow threshold-row">
        <span class="tx">
          <span class="lb">{{ t("panels.rounds_without_progress") }}</span>
          <span class="ds">{{ t("panels.tool_rounds_in_a_row", { n: defaultRounds }) }}</span>
        </span>
        <div class="threshold-control">
          <span class="unit-field">
            <input type="text" inputmode="numeric" :aria-label="t('panels.rounds_without_progress')" data-action-keydown="progress-watch.rounds"
              data-action-change="progress-watch.rounds" :value="rounds" :disabled="progress.busy.value"
              @input="onRoundsInput" @blur="commitRounds" @keydown="blurOnEnter" />
            <i>{{ t("panels.rounds") }}</i>
          </span>
        </div>
      </div>
      <div class="lrow threshold-row">
        <span class="tx">
          <span class="lb">{{ t("panels.input_token_backstop") }}</span>
          <span class="ds">{{ t("panels.a_model_independent_backstop_the", { n: defaultMultiple }) }}</span>
        </span>
        <div class="threshold-control">
          <span class="unit-field">
            <input type="text" inputmode="numeric" :aria-label="t('panels.input_token_backstop')" data-action-keydown="progress-watch.token-multiple"
              data-action-change="progress-watch.token-multiple" :value="multiple" :disabled="progress.busy.value"
              @input="onMultipleInput" @blur="commitMultiple" @keydown="blurOnEnter" />
            <i>{{ t("panels.window") }}</i>
          </span>
        </div>
      </div>
    </div>
    <div v-if="progress.error.value" class="why" role="alert">{{ progress.error.value }}</div>
  </SectionCard>

  <SectionCard :title="t('settings.collaboration')">
    <SettingRow id="session-ref" :title="t('composer.size_of_a_referenced_conversation')" :desc="t('settings.most_a_quote_may_carry')">
      <div v-if="!refs.state.value" class="empty">{{ t("composer.the_reference_settings_could_not") }}</div>
      <template v-else>
        <span class="unit-field">
          <input type="text" inputmode="numeric" :aria-label="t('composer.largest_quote')" data-action-keydown="session-ref.tokens"
            data-action-change="session-ref.tokens" :value="refDraft" :disabled="refs.busy.value"
            @input="onRefInput" @blur="commitRef" @keydown="blurOnEnter" />
          <i>{{ t("settings.token_unit") }}</i>
        </span>
        <button type="button" class="spill" data-action="session-ref.reset" :disabled="refs.busy.value || refAtDefault" @click="resetRef">
          {{ t("composer.restore_default") }}
        </button>
      </template>
    </SettingRow>
    <div v-if="refs.error.value" class="why" role="alert">{{ refs.error.value }}</div>

    <SettingRow id="subagent-claims" :title="t('session.sub_agent_write_scope')" :desc="t('session.each_sub_agents_write_scope')"
      :more="t('subagent.a_sub_agent_that_names')">
      <div v-if="!claims.state.value" class="empty">{{ t("subagent.could_not_read_the_sub") }}</div>
      <div v-else class="seg" data-text="true" role="radiogroup" :aria-label="t('session.sub_agent_write_scope')">
        <button v-for="opt in CLAIM_OPTIONS" :key="opt.id" type="button" role="radio" data-action="subagent-claims.optimistic"
          :data-value="opt.id" :aria-checked="claimsValue === opt.id" :disabled="claims.busy.value" @click="pickClaims(opt.id)">
          {{ t(opt.label) }}
        </button>
      </div>
    </SettingRow>
    <div v-if="claims.error.value" class="why" role="alert">{{ claims.error.value }}</div>
  </SectionCard>

  <SectionCard :title="t('settings.automation')">
    <Hooks :settings-props="settingsProps" />
    <AgentScheduledRuns :port="settingsProps.port" :on-changed="settingsProps.onChanged" />
  </SectionCard>

  <!-- Who the agent is, and what the person's own sessions may say to each
       other: pages of their own as well as blocks on this one, drawn from the
       files that answer for "persona" and "peers". -->
  <PersonaSection :settings-props="settingsProps" />
  <PeersSection :settings-props="settingsProps" />
</template>
