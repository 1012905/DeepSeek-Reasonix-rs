<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { current, t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { FeedbackLinkedInstall } from "../port/feedback";
import type { AgentPort } from "../port/port";
import { boardNameProblem, boardPrefill, markBoardOfferSeen } from "./boardname";
import FeedbackBoardJoin from "./FeedbackBoardJoin.vue";
import { linkFailure } from "./feedbackfailure";

const props = defineProps<{
  port: AgentPort;
  // Linking names this install, which only exists once a report has been sent.
  hasReport: boolean;
  // The board changed under the page: the link step may have listed the person.
  onBoardChanged?: () => void;
}>();

const day = (iso: string): string => {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? "" : at.toLocaleDateString(current() === "zh" ? "zh-CN" : "en", { year: "numeric", month: "short", day: "numeric" });
};

const signedIn = ref<boolean | null>(null);
const links = ref<FeedbackLinkedInstall[] | null>(null);
const busy = ref(false);
const problem = ref<string | null>(null);
const asking = ref(false);
const join = ref(true);
const name = ref("");

const load = async () => {
  try {
    const state = await props.port.account();
    signedIn.value = state.signedIn && !state.expired;
    if (!state.signedIn || state.expired) {
      links.value = null;
      return;
    }
    links.value = await props.port.feedbackLinks();
    problem.value = null;
  } catch (e) {
    problem.value = linkFailure(e).message;
  }
};
watch(() => props.port, () => void load(), { immediate: true });

const run = async (act: () => Promise<void>) => {
  busy.value = true;
  problem.value = null;
  try {
    await act();
    links.value = await props.port.feedbackLinks();
  } catch (e) {
    problem.value = linkFailure(e).message;
  } finally {
    busy.value = false;
  }
};

const ask = () => {
  asking.value = true;
  join.value = true;
  problem.value = null;
  props.port.feedbackEnv(document.documentElement.lang).then((e) => { if (!name.value) name.value = boardPrefill(e.displayName); }, () => {});
};

// Linking happens on confirm; the board name rides along only when it is
// ticked and valid, and a refusal of the name leaves the link in place.
const confirm = async () => {
  busy.value = true;
  problem.value = null;
  try {
    await props.port.linkFeedbackAccount();
  } catch (e) {
    problem.value = linkFailure(e).message;
    busy.value = false;
    return;
  }
  markBoardOfferSeen();
  asking.value = false;
  try {
    links.value = await props.port.feedbackLinks();
    if (join.value && boardNameProblem(name.value) === null) await props.port.joinFeedbackBoard(name.value.trim());
  } catch (e) {
    problem.value = join.value ? t("feedback.linked_but_joining_the_board", { why: reason(e) }) : linkFailure(e).message;
  } finally {
    busy.value = false;
    props.onBoardChanged?.();
  }
};

const setJoin = (on: boolean) => { join.value = on; };
const setName = (value: string) => { name.value = value; };
const unlinkThis = () => void run(() => props.port.unlinkFeedbackAccount());
const unlinkOther = (id: string) => void run(() => props.port.unlinkFeedbackInstall(id));

const here = computed(() => links.value?.find((l) => l.current) ?? null);
const others = computed(() => (links.value ?? []).filter((l) => !l.current));
const canLink = computed(() => signedIn.value === true && props.hasReport && !busy.value);
</script>

<template>
  <section v-if="signedIn !== null" class="fbk-link" :aria-label="t('feedback.account_link')">
    <div class="fbk-link-hd">
      <span class="fbk-link-name">{{ t("feedback.account_link") }}</span>
      <span class="fbk-hint">{{ t("feedback.once_linked_this_computers_feedback") }}</span>
    </div>
    <p v-if="!signedIn" class="fbk-hint">{{ t("feedback.sign_in_to_your_account") }}</p>
    <p v-if="signedIn && !hasReport && !here" class="fbk-hint">{{ t("feedback.you_can_link_this_computer") }}</p>
    <div class="fbk-link-row">
      <template v-if="here">
        <span class="fbk-chip" data-status="recorded">{{ t("feedback.this_computer_is_linked") }} · {{ day(here.linkedAt) }}</span>
        <button type="button" class="btn sm" data-action="feedback.account-unlink" :disabled="busy" @click="unlinkThis">
          {{ t("feedback.unlink_2") }}
        </button>
      </template>
      <button v-else type="button" class="btn sm" data-action="feedback.account-link" :disabled="!canLink || asking" @click="ask">
        {{ t("feedback.link_this_computer_to_my") }}
      </button>
    </div>
    <div v-if="asking && !here" class="fbk-link-confirm" role="group" :aria-label="t('feedback.confirm_linking')">
      <FeedbackBoardJoin :checked="join" :name="name" :disabled="busy" :on-checked="setJoin" :on-name="setName" />
      <div class="fbk-link-row">
        <button type="button" class="btn sm" data-primary="true" data-action="feedback.account-link-confirm" :disabled="busy || (join && boardNameProblem(name) !== null)" @click="confirm">
          {{ t("common.confirm") }}
        </button>
        <button type="button" class="btn sm" data-action="feedback.account-link-cancel" :disabled="busy" @click="asking = false">
          {{ t("common.cancel") }}
        </button>
      </div>
    </div>
    <ul v-if="others.length > 0" class="fbk-link-list" :aria-label="t('feedback.other_computers_on_this_account')">
      <li v-for="l in others" :key="l.id">
        <code class="fbk-code">{{ l.id }}</code>
        <span class="fbk-meta">{{ day(l.linkedAt) }}</span>
        <button type="button" class="btn sm" data-action="feedback.account-unlink-other" :data-target="l.id" :disabled="busy" @click="unlinkOther(l.id)">
          {{ t("feedback.unlink") }}
        </button>
      </li>
    </ul>
    <p v-if="problem" class="fbk-hint" role="alert" data-tone="error">{{ problem }}</p>
  </section>
</template>
