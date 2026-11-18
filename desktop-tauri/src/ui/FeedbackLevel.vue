<script setup lang="ts">
import { computed, useId } from "vue";
import { current, t } from "../i18n";
import type { FeedbackProfile } from "../port/feedback";
import LevelBadge from "./LevelBadge.vue";

const LEVEL_NAME = ["feedback.new_seed", "feedback.sprout", "feedback.seedling", "feedback.sapling", "feedback.bloom", "feedback.canopy", "feedback.grove"] as const;

const levelName = (level: number): string | null => (Number.isInteger(level) && level >= 0 && level < LEVEL_NAME.length ? t(LEVEL_NAME[level]!) : null);

const levelLabel = (level: number): string => {
  const name = levelName(level);
  return name ? t("L{n} · {name}", { n: level, name }) : t("L{n}", { n: level });
};

const day = (iso: string): string => new Date(iso).toLocaleDateString(current() === "zh" ? "zh-CN" : "en", { year: "numeric", month: "short", day: "numeric" });

const validDate = (iso: string | null | undefined): iso is string => typeof iso === "string" && !Number.isNaN(Date.parse(iso));

function progressText(p: FeedbackProfile): string {
  if (p.nextLevel === null || p.remaining === null) return t("feedback.n_shipped_the_highest_level", { n: p.adoptedCount });
  const next = levelName(p.nextLevel);
  return next
    ? t("feedback.n_shipped_left_more_to_2", { n: p.adoptedCount, left: p.remaining, name: next })
    : t("feedback.n_shipped_left_more_to", { n: p.adoptedCount, left: p.remaining, level: p.nextLevel });
}

function trustNote(p: FeedbackProfile): string | null {
  if (!validDate(p.trustExpiresAt)) return p.trustState === "lapsed" ? t("feedback.your_higher_limits_have_lapsed") : null;
  switch (p.trustState) {
    case "legacy_active":
      return t("feedback.your_current_higher_limits_stay", { date: day(p.trustExpiresAt) });
    case "active":
      return t("feedback.your_higher_limits_run_until", { date: day(p.trustExpiresAt) });
    case "lapsed":
      return t("feedback.your_higher_limits_have_lapsed");
    default:
      return null;
  }
}

const props = defineProps<{
  profile: FeedbackProfile;
  offline: boolean;
}>();

const limits = computed(() => props.profile.effectiveLimits);
const span = computed(() => (props.profile.nextThreshold === null ? 0 : props.profile.nextThreshold - props.profile.currentThreshold));
const done = computed(() => Math.min(Math.max(props.profile.adoptedCount - props.profile.currentThreshold, 0), span.value));
const next = computed(() => props.profile.nextLevel);
const note = computed(() => trustNote(props.profile));
const said = computed(() => progressText(props.profile));
const fill = computed(() => `${+((done.value / span.value) * 100).toFixed(2)}%`);
const meaning = useId();
</script>

<template>
  <section class="fbk-level" :aria-label="t('feedback.feedback_level')" :data-level="profile.level" :data-trust="profile.trustState" :data-stale="offline ? '' : undefined">
    <div class="fbk-level-hd">
      <LevelBadge :level="profile.level" decorative :size="32" />
      <div class="fbk-level-id">
        <span class="fbk-level-name">{{ levelLabel(profile.level) }}</span>
        <span class="fbk-level-progress" :aria-describedby="meaning">{{ said }}</span>
      </div>
    </div>
    <div v-if="next !== null && span > 0" class="fbk-level-track">
      <div
        class="fbk-level-bar"
        role="progressbar"
        :aria-label="t('feedback.progress_to_the_next_level')"
        :aria-valuemin="0"
        :aria-valuenow="done"
        :aria-valuemax="span"
        :aria-valuetext="said"
      >
        <i class="fbk-level-fill" :style="{ width: fill }" />
      </div>
      <span class="fbk-level-next">
        <LevelBadge :level="next" locked decorative :size="24" />
        <span>{{ levelName(next) ?? `L${next}` }}</span>
      </span>
    </div>
    <p class="fbk-level-meaning" :id="meaning">{{ t("feedback.shipped_means_your_report_was") }}</p>
    <p class="fbk-level-limits">
      {{ t("feedback.up_to_h_reports_an", { h: limits.reportsPerHour, d: limits.reportsPerDay, r: limits.repliesPerHour }) }}
    </p>
    <p v-if="note" class="fbk-level-note">{{ note }}</p>
    <p v-if="offline && validDate(profile.observedAt)" class="fbk-level-stale">{{ t("feedback.this_is_the_last_confirmed", { date: day(profile.observedAt) }) }}</p>
  </section>
</template>
