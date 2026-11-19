<script lang="ts">
import type { FeedbackItem, FeedbackMine as Mine } from "../port/feedback";

// What this window last read, per port, so opening the page again draws the list
// it already has and refreshes behind it instead of showing one waiting line.
const remembered = new WeakMap<object, Mine>();

const newest = (i: FeedbackItem) => Math.max(0, ...i.replies.map((r) => r.id));
</script>

<script setup lang="ts">
import { computed, h, ref, watch, type VNodeChild } from "vue";
import { t } from "../i18n";
import { FEEDBACK_REPO_ISSUES, type FeedbackLimits } from "../port/feedback";
import type { AgentPort } from "../port/port";
import FeedbackDetail from "./FeedbackDetail.vue";
import FeedbackFilter from "./FeedbackFilter.vue";
import { inGroup, type FeedbackGroup } from "./feedbackgroups";
import { feedbackFailure, type FeedbackFailure } from "./feedbackfailure";
import StatusChips, { CATEGORY_LABEL, shortDay, snippet } from "./feedbackstatus.vue";
import StudioIcon from "./StudioIcon.vue";

const props = withDefaults(defineProps<{
  port: AgentPort;
  onFile: (url: string) => void;
  // How many reports still want attention once this page has shown them.
  onUnread?: (n: number) => void;
}>(), {});

const mine = ref<Mine | null>(remembered.get(props.port) ?? null);
const failure = ref<FeedbackFailure | null>(null);
const loading = ref(true);
const fresh = ref<Record<string, number>>({});
const group = ref<FeedbackGroup>("all");
const chosen = ref<string | null>(null);
const limits = ref<FeedbackLimits | null>(null);
const said = ref("");

// Unsent reply text per report, so moving to another report and back keeps it.
const drafts = new Map<string, string>();
let heldItems: FeedbackItem[] = [];
let pending = new Set<string>();

const tell = () => props.onUnread?.(heldItems.filter((i) => i.needsInput || pending.has(i.receipt)).length);

const markSeen = async (item: FeedbackItem) => {
  const upTo = newest(item);
  await props.port.feedbackSeen(item.receipt, upTo);
  const current = heldItems.find((i) => i.receipt === item.receipt);
  if (current && newest(current) <= upTo) pending.delete(item.receipt);
  tell();
};

const load = () => {
  loading.value = true;
  props.port
    .myFeedback()
    .then((m) => {
      remembered.set(props.port, m);
      mine.value = m;
      failure.value = null;
      const next = { ...fresh.value };
      for (const i of m.items) if (i.unreadReplies > 0) next[i.receipt] = Math.max(next[i.receipt] ?? 0, i.unreadReplies);
      fresh.value = next;
      heldItems = m.items;
      pending = new Set(m.items.filter((i) => i.unreadReplies > 0).map((i) => i.receipt));
      tell();
      const fresher = m.items.filter((i) => i.unreadReplies > 0).length;
      if (fresher > 0) said.value = t("feedback.n_of_your_reports_have", { n: fresher });
    })
    .catch((e) => { failure.value = feedbackFailure(e); })
    .finally(() => { loading.value = false; });
};

watch(() => props.port, load, { immediate: true });

watch(() => props.port, (port, _was, onCleanup) => {
  let live = true;
  port.feedbackEnv(document.documentElement.lang).then((e) => { if (live) limits.value = e.limits; }, () => {});
  onCleanup(() => { live = false; });
}, { immediate: true });

// A question waiting on the reporter comes first; otherwise the service's order.
const items = computed(() => (mine.value ? [...mine.value.items].sort((a, b) => Number(b.needsInput) - Number(a.needsInput)) : []));
const visible = computed(() => items.value.filter((i) => inGroup(i, group.value, fresh.value[i.receipt] ?? 0)));
const shown = computed(() => visible.value.find((i) => i.receipt === chosen.value) ?? visible.value[0]);

watch(shown, (now) => {
  if (now && pending.has(now.receipt)) void markSeen(now).catch(() => {});
}, { immediate: true });

const sent = (receipt: string) => {
  said.value = t("feedback.reply_sent");
  chosen.value = receipt;
  load();
};

const withdrawn = () => {
  said.value = t("feedback.the_report_was_withdrawn");
  load();
};

const setGroup = (to: FeedbackGroup) => { group.value = to; };
const rowLabel = (item: FeedbackItem) => `${item.receipt} ${snippet(item.titleSnippet)}`;

const issue = (n: number): VNodeChild => {
  if (!Number.isSafeInteger(n) || n <= 0) return `#${n}`;
  const url = FEEDBACK_REPO_ISSUES + String(n);
  return h("a", { href: url, rel: "noopener noreferrer", "data-action": "feedback.link", onClick: (e: MouseEvent) => { e.preventDefault(); props.onFile(url); } }, `#${n}`);
};
</script>

<template>
  <div class="fbk-mine" :aria-busy="loading" :data-empty="mine && items.length === 0 ? '' : undefined">
    <p class="sr-only" role="status" aria-live="polite">{{ said }}</p>
    <div class="fbk-side">
      <FeedbackFilter v-if="items.length > 1" :items="items" :fresh="fresh" :group="group" :on-pick="setGroup" />

      <div v-if="mine?.offline" class="fbk-note" role="status" data-tone="info">
        <StudioIcon name="warning" />
        <span>{{ t("feedback.cannot_reach_the_feedback_service") }}</span>
      </div>

      <div v-if="failure" class="fbk-note" role="alert" data-tone="error">
        <StudioIcon name="warning" />
        <span>{{ failure.message }}</span>
        <button type="button" class="btn sm" data-action="feedback.retry-mine" @click="load">{{ t("common.retry") }}</button>
      </div>

      <p v-if="!mine && !failure" class="fbk-empty" role="status">{{ t("feedback.loading_your_feedback") }}</p>

      <p v-if="items.length > 1 && visible.length === 0" class="fbk-empty">{{ t("feedback.no_reports_in_this_group") }}</p>

      <ul v-if="items.length > 0" class="fbk-list">
        <li v-for="item in items" :key="item.receipt" class="fbk-item" :hidden="!inGroup(item, group, fresh[item.receipt] ?? 0)" :data-receipt="item.receipt"
          :data-status="item.status" :data-stale="item.statusUnavailable ? '' : undefined" :data-fresh="(fresh[item.receipt] ?? 0) > 0 ? '' : undefined">
          <button type="button" class="fbk-row" data-action="feedback.pick" :data-target="item.receipt"
            :aria-current="item.receipt === shown?.receipt ? 'true' : undefined" :aria-label="rowLabel(item)"
            @click="chosen = item.receipt">
            <span class="fbk-row-title" dir="auto">{{ snippet(item.titleSnippet) }}</span>
            <span class="fbk-row-meta">{{ t(CATEGORY_LABEL[item.category]) }} · {{ shortDay(item.createdAt) }}</span>
            <span class="fbk-row-chips"><StatusChips :item="item" /></span>
          </button>
        </li>
      </ul>

      <div class="fbk-footnotes">
        <p class="fbk-hint">
          {{ t("feedback.refreshed_each_time_you_open") }}
          <button type="button" class="fbk-linkish" data-action="feedback.refresh" :disabled="loading" @click="load">{{ t("feedback.refresh_list") }}</button>
        </p>
        <p v-if="items.some((i) => i.otherInstall)" class="fbk-hint">{{ t("feedback.read_marks_are_kept_on") }}</p>
        <p v-if="items.some((i) => i.statusUnavailable)" class="fbk-hint" role="status">{{ t("feedback.some_reports_were_sent_under") }}</p>
      </div>
    </div>
    <div class="fbk-main">
      <FeedbackDetail v-if="shown" :port="port" :item="shown" :level="mine?.profile?.level ?? null" :fresh="fresh[shown.receipt] ?? 0" :limits="limits"
        :offline="mine?.offline ?? false" :issue="issue" :on-file="onFile" :on-sent="sent" :drafts="drafts"
        :on-withdrawn="withdrawn" :on-stale="load" />
      <p v-else-if="mine && items.length === 0" class="fbk-empty">{{ t("feedback.no_feedback_sent_yet_once") }}</p>
    </div>
  </div>
</template>
