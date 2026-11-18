<script setup lang="ts">
import { computed, toValue } from "vue";
import { FEEDBACK_RETIRED } from "../feedbackchannel";
import { t } from "../i18n";
import { FeedbackBadge } from "./feedbackentry.vue";
import { useDestinations, type Mark } from "./navdest";
import { ICON, type Section } from "./prefsnav";
import StudioIcon from "./StudioIcon.vue";
import UpdateDot from "./UpdateDot.vue";

// Every entry goes somewhere that exists. A rail of icons that open nothing is
// the cheapest thing to draw and the most expensive thing to keep.
const HOME: [Mark, string, string | null] = ["chat", "common.session", null];

const FOOT: [Mark, string, string, string?][] = [
  ["account", "settings.account", "account"],
  ["advanced", "common.settings", "", "chrome.settings"],
];

const props = defineProps<{
  at: string | null;
  onGo: (sec?: string) => void;
  onFiles: () => void;
  onHome: () => void;
  /** The conversation item already shows the conversation, so pressing it again
   *  does what the rail beside it can: open or close the workspace column. */
  onRail: () => void;
  onFeedback: () => void;
  /** A new session in the window's folder, the same action as the sidebar's. */
  onNew: () => void;
  feedbackUnread: number;
  /** False keeps the column mounted but out of the way and out of reach. */
  shown: boolean;
}>();

const destinations = useDestinations();
const rows = computed(() => [HOME, ...toValue(destinations)] as [Mark, string, string | null, string?][]);

const iconOf = (key: Mark) => ICON[(key === "advanced" ? "common" : key) as Section];

const go = (key: Mark, sec: string | null) => {
  if (key === "files") return props.onFiles();
  if (sec === null) return props.at === null ? props.onRail() : props.onHome();
  return props.onGo(key === "ext" ? "extensions" : key === "usage" ? "spend" : sec || undefined);
};
</script>

<template>
  <nav class="nav" :aria-label="t('settings.main_navigation')" :aria-hidden="shown ? undefined : true" :inert="!shown">
    <div class="navgrp">
      <button class="navbtn" data-action="session.new" :aria-label="t('common.start_a_new_session')" @click="onNew()">
        <StudioIcon name="plus" aria-hidden="true" />
        <span class="navhint" role="tooltip">{{ t("common.start_a_new_session") }}</span>
      </button>
      <button
        v-for="[key, label, sec, action] in rows"
        :key="key"
        class="navbtn"
        :data-action="key === 'account' ? 'chrome.account' : action"
        :aria-label="t(label)"
        :aria-current="(sec ?? null) === at ? 'page' : undefined"
        @click="go(key, sec)"
      >
        <StudioIcon v-if="key === 'files'" name="file" />
        <StudioIcon v-else-if="key === 'chat'" name="chat" />
        <component :is="iconOf(key)" v-else />
        <UpdateDot v-if="key === 'advanced'" />
        <span class="navhint" role="tooltip">{{ t(label) }}</span>
      </button>
    </div>
    <div class="navgrp foot">
      <button v-if="!FEEDBACK_RETIRED" class="navbtn" data-action="feedback.open" @click="onFeedback()">
        <StudioIcon name="feedback" aria-hidden="true" />
        <span class="navhint" role="tooltip">{{ t("common.send_feedback") }}</span>
        <FeedbackBadge :unread="feedbackUnread" />
      </button>
      <button
        v-for="[key, label, sec, action] in FOOT"
        :key="key"
        class="navbtn"
        :data-action="key === 'account' ? 'chrome.account' : action"
        :aria-label="t(label)"
        :aria-current="(sec ?? null) === at ? 'page' : undefined"
        @click="go(key, sec)"
      >
        <StudioIcon v-if="key === 'files'" name="file" />
        <StudioIcon v-else-if="key === 'chat'" name="chat" />
        <component :is="iconOf(key)" v-else />
        <UpdateDot v-if="key === 'advanced'" />
        <span class="navhint" role="tooltip">{{ t(label) }}</span>
      </button>
    </div>
  </nav>
</template>