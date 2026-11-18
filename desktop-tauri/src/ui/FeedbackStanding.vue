<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import type { AgentPort } from "../port/port";
import type { FeedbackMine } from "../port/feedback";
import FeedbackBoard from "./FeedbackBoard.vue";
import FeedbackLevel from "./FeedbackLevel.vue";
import FeedbackLink from "./FeedbackLink.vue";

/** Level, account and leaderboard as one line in the dialog's title bar, opened on
 *  request; nothing behind it is asked for until it is opened. */
const props = defineProps<{ port: AgentPort; mine: FeedbackMine; slot?: HTMLElement | null }>();

const open = ref(false);
const boardTick = ref(0);
const p = computed(() => props.mine.profile ?? null);
</script>

<template>
  <Teleport :to="slot ?? 'body'" :disabled="!slot">
    <div class="fbk-standing">
      <button type="button" class="fbk-standing-btn" data-action="feedback.account-section" :aria-expanded="open"
        :title="t('feedback.level_account_and_leaderboard')" @click="open = !open">
        {{ p ? t("feedback.standing", { level: p.level, n: p.adoptedCount }) : t("feedback.level_account_and_leaderboard") }}
      </button>
      <div v-if="open" class="fbk-standing-pop">
        <FeedbackLevel v-if="p" :profile="p" :offline="mine.offline" />
        <p v-else-if="mine.items.length > 0" class="fbk-hint fbk-level-gone">{{ t("feedback.your_level_cannot_be_shown") }}</p>
        <FeedbackLink :port="port" :has-report="mine.items.length > 0" :on-board-changed="() => (boardTick += 1)" />
        <FeedbackBoard :port="port" :reload="boardTick" />
      </div>
    </div>
  </Teleport>
</template>
