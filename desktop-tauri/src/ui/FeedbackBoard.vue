<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { FeedbackBoard as Board, FeedbackBoardMe } from "../port/feedback";
import type { AgentPort } from "../port/port";
import { boardNameProblem, boardOfferSeen, boardPrefill, markBoardOfferSeen } from "./boardname";

const props = withDefaults(defineProps<{ port: AgentPort; reload?: number }>(), { reload: 0 });

const newest = (a: string, b: string): number => {
  const pa = a.replace(/^v/, "").split(".").map(Number);
  const pb = b.replace(/^v/, "").split(".").map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pb[i] ?? 0) - (pa[i] ?? 0);
  return 0;
};

const failure = (e: unknown): string => reason(e);

const all = ref<Board | null>(null);
const board = ref<Board | null>(null);
const release = ref("");
const signedIn = ref(false);
const me = ref<FeedbackBoardMe | null>(null);
const draft = ref("");
const busy = ref(false);
const problem = ref("");
const linked = ref(false);
const offered = ref(!boardOfferSeen());
const offerName = ref("");

const load = async () => {
  try {
    const [first, account] = await Promise.all([props.port.feedbackBoard(), props.port.account().catch(() => null)]);
    all.value = first;
    board.value = first;
    const on = !!account && account.signedIn && !account.expired;
    signedIn.value = on;
    if (!on) {
      linked.value = false;
      return;
    }
    const [mine, links, env] = await Promise.all([
      props.port.feedbackBoardMe().catch(() => null),
      props.port.feedbackLinks().catch(() => []),
      props.port.feedbackEnv(document.documentElement.lang).catch(() => null),
    ]);
    me.value = mine;
    linked.value = links.some((l) => l.current);
    if (env) offerName.value = offerName.value || boardPrefill(env.displayName);
  } catch {
    all.value = null;
  }
};
watch([() => props.port, () => props.reload], () => { void load(); }, { immediate: true });

const versions = computed(() => [...new Set((all.value?.entries ?? []).flatMap((e) => e.releases.map((r) => r.version)))].sort(newest));
const ranked = computed(() => board.value?.view === "release");
const typed = computed(() => draft.value.trim());

const pick = async (next: string) => {
  release.value = next;
  try {
    board.value = await props.port.feedbackBoard(next || undefined);
  } catch (e) {
    problem.value = failure(e);
  }
};

const run = async (act: () => Promise<FeedbackBoardMe>) => {
  busy.value = true;
  problem.value = "";
  try {
    me.value = await act();
    all.value = await props.port.feedbackBoard();
    board.value = await props.port.feedbackBoard(release.value || undefined);
  } catch (e) {
    problem.value = failure(e);
  } finally {
    busy.value = false;
  }
};

const onRelease = (e: Event) => void pick((e.target as HTMLSelectElement).value);
const onOfferName = (e: Event) => (offerName.value = (e.target as HTMLInputElement).value);
const onDraft = (e: Event) => (draft.value = (e.target as HTMLInputElement).value);
const joinOffer = () => { markBoardOfferSeen(); offered.value = false; void run(() => props.port.joinFeedbackBoard(offerName.value.trim())); };
const dismissOffer = () => { markBoardOfferSeen(); offered.value = false; };
const joinTyped = () => void run(() => props.port.joinFeedbackBoard(typed.value));
const leave = () => void run(() => props.port.leaveFeedbackBoard());
</script>

<template>
  <section v-if="all?.available && board" class="fbk-board" :aria-label="t('feedback.contribution_board')">
    <div class="fbk-link-hd">
      <span class="fbk-link-name">{{ t("feedback.contribution_board") }}</span>
      <span class="fbk-hint">{{ t("feedback.for_people_whose_feedback_was") }}</span>
    </div>
    <label class="fbk-board-pick">
      <span class="fbk-hint">{{ t("common.version") }}</span>
      <select :aria-label="t('common.version')" data-action-change="feedback.board-release" :value="release" @change="onRelease">
        <option value="">{{ t("feedback.all_releases") }}</option>
        <option v-for="v in versions" :key="v" :value="v">{{ v }}</option>
      </select>
    </label>
    <p v-if="board.entries.length === 0" class="fbk-hint">{{ t("feedback.nobody_is_listed_for_this") }}</p>
    <ol v-else class="fbk-board-list" :data-ranked="ranked ? '' : undefined">
      <li v-for="e in board.entries" :key="e.displayName">
        <span v-if="ranked" class="fbk-board-rank" :aria-label="t('feedback.place_n', { n: e.rank ?? 0 })">{{ e.rank }}</span>
        <span class="fbk-board-name">{{ e.displayName }}</span>
        <span class="fbk-board-credits">{{ t("feedback.n_adopted", { n: e.credits }) }}</span>
        <span v-if="!ranked" class="fbk-board-where">
          <span v-for="r in e.releases.slice(0, 3)" :key="r.version" class="fbk-chip" data-status="recorded">{{ r.version }}{{ r.count > 1 ? ` ×${r.count}` : "" }}</span>
          <span v-if="e.releases.length > 3" class="fbk-meta">+{{ e.releases.length - 3 }}</span>
        </span>
      </li>
    </ol>
    <p class="fbk-hint">{{ t("feedback.a_ranking_depends_on_what") }}</p>

    <div v-if="signedIn && linked && offered && !me?.listed && !me?.delisted" class="fbk-board-offer" role="group" :aria-label="t('feedback.join_the_contribution_board')">
      <span class="fbk-link-name">{{ t("feedback.join_the_contribution_board_2") }}</span>
      <p class="fbk-hint">{{ t("feedback.this_computer_is_linked_to") }}</p>
      <div class="fbk-link-row">
        <input class="fbk-board-name-field" data-action-change="feedback.board-name" :aria-label="t('feedback.name_shown_on_the_board')" :value="offerName" :maxlength="40" :placeholder="t('feedback.name_shown_on_the_board')" @input="onOfferName" />
        <button type="button" class="btn sm" data-primary="true" data-action="feedback.board-join" :disabled="busy || boardNameProblem(offerName) !== null" @click="joinOffer">{{ t("feedback.join_the_board") }}</button>
        <button type="button" class="btn sm" data-action="feedback.board-offer-dismiss" :disabled="busy" @click="dismissOffer">{{ t("common.no_thanks") }}</button>
      </div>
    </div>
    <div class="fbk-board-me">
      <span class="fbk-link-name">{{ t("feedback.my_place_on_the_board") }}</span>
      <p v-if="!signedIn" class="fbk-hint">{{ t("feedback.sign_in_from_settings_first") }}</p>
      <p v-else-if="me?.delisted" class="fbk-hint" data-tone="warn">{{ t("feedback.a_maintainer_removed_this_account") }}</p>
      <div v-else-if="me?.listed" class="fbk-link-row">
        <span class="fbk-chip" data-status="recorded">{{ t("feedback.shown_on_the_board_as") }} · {{ me.displayName }}</span>
        <button type="button" class="btn sm" data-action="feedback.board-leave" :disabled="busy" @click="leave">{{ t("feedback.leave_the_board") }}</button>
      </div>
      <template v-else>
        <p class="fbk-hint">{{ t("feedback.once_you_join_the_board") }}</p>
        <div class="fbk-link-row">
          <input class="fbk-board-name-field" data-action-change="feedback.board-name" :aria-label="t('feedback.name_shown_on_the_board')" :value="draft" :maxlength="30" :placeholder="t('feedback.name_shown_on_the_board')" @input="onDraft" />
          <button type="button" class="btn sm" data-primary="true" data-action="feedback.board-join" :disabled="busy || typed.length < 2" @click="joinTyped">{{ t("feedback.join_the_board") }}</button>
        </div>
        <p v-if="typed.length >= 2" class="fbk-hint" aria-live="polite">{{ t("feedback.it_will_be_shown_on", { name: typed }) }}</p>
      </template>
    </div>
    <p v-if="problem" class="fbk-hint" role="alert" data-tone="error">{{ problem }}</p>
  </section>
</template>