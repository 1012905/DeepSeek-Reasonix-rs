<script lang="ts">
import type { Queue as QueueSnapshot, QueueItem } from "../port/port";
import { t } from "../i18n";
import { BLOCK_WHY } from "../i18n/queue_why";

// The kernel has taken the line: it is in the transcript now, and the entry is
// swept only when the whole turn ends. This strip is what has not happened yet,
// so a taken entry leaves it rather than standing there inert as a second copy.
const taken = (s: QueueItem["state"]) => s === "steer_consumed" || s === "running";

// A host continuation — a finished background job asking the session to pick
// the work back up — is not the person's and belongs with the work it came
// from. It appears here only when it needs a decision.
const theirs = (it: QueueItem) => it.origin !== "host" || it.state === "blocked" || it.state === "uncertain";

// Codes whose entry the kernel knows never reached the model, so sending it
// again cannot repeat an effect. Every other uncertain code may have run.
const RETRY_SAFE = new Set(["steer_unapplied"]);
const retryable = (it: QueueItem) =>
  it.state === "blocked" || (it.state === "uncertain" && !!it.blockCode && RETRY_SAFE.has(it.blockCode));
// Only plain text can become guidance: a picture or a frozen file reference
// travels with a whole turn.
const steerable = (it: QueueItem) =>
  it.origin !== "host" && it.state === "queued" && it.intent === "followup" && !it.images && !it.refs?.length;

/** What the strip draws: the lines nobody has acted on yet. The header's count
 *  of guidance in flight reads this too, so it can never name a line the strip
 *  is not showing. */
export const waiting = (queue: QueueSnapshot | null): QueueItem[] =>
  queue ? queue.items.filter((it) => !taken(it.state) && theirs(it)) : [];

// Each arm calls t() with its own literal: the catalogue is built by reading
// these call sites, and a table of strings looked up later is invisible to it —
// which is how a row would render Chinese inside an English window.
function label(it: QueueItem, n: number): string {
  if (it.origin === "host") {
    if (it.state === "blocked") return t("composer.background_blocked");
    if (it.state === "uncertain") return t("composer.background_uncertain");
    return t("composer.background_job_follow_on");
  }
  switch (it.state) {
    case "steer_accepted":
      return t("composer.steer_accepted");
    case "blocked":
      return t("composer.blocked");
    case "uncertain":
      return t("composer.uncertain");
    default:
      if (it.images && it.state === "queued") return t("composer.has_an_image_sent_when");
      if (it.intent === "steer") return t("composer.steer_queued");
      return `${t("common.queued")} ${n}`;
  }
}
</script>

<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  queue: QueueSnapshot | null;
  // A turn holding the session is what these lines wait on, and what "send now"
  // has to interrupt. Without it the action is meaningless: an idle queue
  // dispatches on its own.
  running: boolean;
  // Entries whose guidance already stands in the transcript as its own bubble.
  shown?: ReadonlySet<string>;
  onSendNow: (item: QueueItem) => void;
  onToSteer: (item: QueueItem) => void;
  onCancel: (id: string) => void | Promise<void>;
  onRetry: (id: string) => void;
  onPause: (paused: boolean) => void;
}>();

const items = computed(() => waiting(props.queue).filter((it) => !props.shown?.has(it.id)));
const sendable = computed(() => props.running && !props.queue?.paused);
const whyOf = (it: QueueItem) => {
  const coded = it.blockCode ? BLOCK_WHY[it.blockCode] : undefined;
  return coded ? t(coded) : it.blockReason;
};
const held = (it: QueueItem) => it.state === "blocked" || it.state === "uncertain";
</script>

<template>
  <div v-if="queue && items.length > 0" class="queue" role="group" :aria-label="t('composer.pending')">
    <div v-for="(it, i) in items" :key="it.id" class="qi" :data-state="it.state" :title="whyOf(it) || undefined">
      <span class="ql" :data-tone="held(it) ? 'warn' : undefined" :title="it.state === 'steer_accepted' ? t('composer.goes_in_at_the_next') : it.origin === 'host' ? t('composer.background_job_follow_on') : undefined">
        {{ label(it, i + 1) }}
      </span>
      <span class="pv">{{ it.preview }}</span>
      <span v-if="held(it) && whyOf(it)" class="qwhy" :title="whyOf(it)">{{ whyOf(it) }}</span>
      <span class="qacts">
        <button v-if="queue.paused && i === 0 && !queue.readonly" class="qb" data-action="queue.pause" @click="onPause(false)">{{ t("composer.release") }}</button>
        <button v-if="sendable && steerable(it)" class="qb" data-action="queue.to-steer" :data-target="it.id" @click="onToSteer(it)">
          {{ t("composer.make_it_a_steer") }}
        </button>
        <button v-if="sendable && (it.state === 'queued' || it.state === 'steer_accepted')" class="qb" data-action="queue.send-now" :data-target="it.id" :title="t('composer.stop_the_current_response_and')" @click="onSendNow(it)">
          {{ t("composer.send_now") }}
        </button>
        <button v-if="retryable(it)" class="qb" data-action="queue.retry" :data-target="it.id" :title="t('common.retry')" @click="onRetry(it.id)">
          {{ t("common.retry") }}
        </button>
        <button v-if="!queue.readonly" class="qb" data-action="queue.cancel" :data-target="it.id" :title="t('composer.take_it_back')" @click="onCancel(it.id)">
          {{ t("composer.withdraw") }}
        </button>
      </span>
    </div>
  </div>
</template>