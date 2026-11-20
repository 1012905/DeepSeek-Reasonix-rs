<script lang="ts">
import { defineComponent, h, type PropType } from "vue";

const encoder = new TextEncoder();

// Used only when the limit could not be read; the kernel still refuses an oversized reply.
const REPLY_BYTES_FALLBACK = 4096;

export const FeedbackReviewNote = defineComponent({
  name: "FeedbackReviewNote",
  props: { item: { type: Object as PropType<FeedbackItem>, required: true } },
  setup(props) {
    return () => (isUnderReview(props.item)
      ? h("p", { class: "fbk-hint fbk-review", "data-review": "", role: "status" }, t("feedback.a_maintainer_is_reading_this"))
      : null);
  },
});

export const FeedbackReplyBox = defineComponent({
  name: "FeedbackReplyBox",
  props: {
    port: { type: Object as PropType<AgentPort>, required: true },
    item: { type: Object as PropType<FeedbackItem>, required: true },
    limit: { type: Number as PropType<number | null>, default: null },
    limits: { type: Object as PropType<Pick<FeedbackLimits, "images" | "uploadBytes"> | null>, default: null },
    offline: { type: Boolean, default: false },
    onSent: { type: Function as PropType<(receipt: string) => void>, required: true },
    onFile: { type: Function as PropType<(url: string) => void>, required: true },
    onStale: { type: Function as PropType<() => void>, required: true },
    drafts: { type: Object as PropType<Map<string, string>>, default: undefined },
  },
  setup(props) {
    const text = ref(props.drafts?.get(props.item.receipt) ?? "");
    const setText = (next: string) => {
      text.value = next;
      if (next) props.drafts?.set(props.item.receipt, next);
      else props.drafts?.delete(props.item.receipt);
    };
    const sending = ref(false);
    const failure = ref<FeedbackFailure | null>(null);
    const field = ref<HTMLTextAreaElement | null>(null);
    const picker = ref<HTMLInputElement | null>(null);
    const shots = ref<Shot[]>([]);
    const refused = ref<Refused[]>([]);

    const add = async (files: File[]) => {
      const limits = props.limits;
      if (!limits || files.length === 0) return;
      const { taken, refused: no } = admit(shots.value.length, files, limits);
      refused.value = no;
      const read = await Promise.allSettled(taken.map(readShot));
      const ok = read.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
      if (ok.length > 0) shots.value = [...shots.value, ...ok].slice(0, limits.images);
    };

    const send = async () => {
      const max = props.limit ?? REPLY_BYTES_FALLBACK;
      const said = text.value.trim();
      if (sending.value || said === "" || encoder.encode(text.value).length > max) return;
      sending.value = true;
      failure.value = null;
      try {
        await (shots.value.length > 0 ? props.port.replyFeedback(props.item.receipt, said, payload(shots.value)) : props.port.replyFeedback(props.item.receipt, said));
        setText("");
        shots.value = [];
        refused.value = [];
        props.onSent(props.item.receipt);
      } catch (e) {
        const f = replyFailure(e);
        failure.value = f;
        if (f.code === FEEDBACK_CODE.notReplyable) props.onStale();
      } finally {
        sending.value = false;
      }
    };

    return () => {
      const item = props.item;
      if (!canReplyTo(item)) return null;
      const reopens = item.status === "fixed";
      if (props.offline) return h("p", { class: "fbk-hint" }, t("feedback.you_cannot_reply_while_offline"));
      const bytes = encoder.encode(text.value).length;
      const max = props.limit ?? REPLY_BYTES_FALLBACK;
      const over = bytes > max;
      const blocked = sending.value || text.value.trim() === "" || over;
      const id = `fbk-reply-${item.receipt}`;
      const limits = props.limits;
      const said = [
        reopens ? t("feedback.sending_this_reopens_the_report") : null,
        item.issueNumber ? t("feedback.this_report_already_has_a", { n: item.issueNumber }) : t("feedback.sent_to_the_maintainers_only"),
        limits ? t("feedback.screenshots_go_to_the_maintainers") : null,
      ].filter(Boolean).join(" ");
      return h("form", {
        class: "fbk-reply",
        "data-needs": item.needsInput ? "" : undefined,
        "aria-busy": sending.value,
        novalidate: true,
        "data-action-submit": "feedback.reply.send",
        onPaste: (e: ClipboardEvent) => {
          const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
          if (files.length === 0 || !props.limits) return;
          e.preventDefault();
          void add(files);
        },
        onSubmit: (e: Event) => { e.preventDefault(); void send(); },
      }, [
        h("label", { for: id, class: "sr-only" }, item.needsInput
          ? t("feedback.your_reply")
          : t(reopens ? "feedback.tell_us_what_is_still" : "feedback.reply_to_the_maintainers")),
        h("div", { class: "fbk-reply-box" }, [
          h("textarea", {
            id,
            ref: field,
            "data-action": "feedback.reply.body",
            value: text.value,
            rows: 2,
            disabled: sending.value,
            placeholder: t(reopens ? "feedback.tell_us_what_is_still" : "feedback.reply_placeholder"),
            "aria-invalid": over ? true : undefined,
            "aria-describedby": `${id}-hint ${id}-count`,
            onInput: (e: Event) => setText((e.target as HTMLTextAreaElement).value),
          }),
          limits && shots.value.length > 0
            ? h("ul", { class: "fbk-shots" }, shots.value.map((s) => h(FeedbackShot, {
                key: s.id,
                shot: s,
                disabled: sending.value,
                onRemove: () => { shots.value = shots.value.filter((x) => x.id !== s.id); },
              })))
            : null,
        ]),
        h("p", { id: `${id}-hint`, class: "fbk-hint fbk-reply-hint" }, said),
        limits && refused.value.length > 0
          ? h("ul", { class: "fbk-refused", role: "status" }, refused.value.map((r, i) => h("li", { key: i },
              t(REFUSED_LABEL[r.why], { name: r.name, size: megabytes(limits.uploadBytes), n: limits.images }))))
          : null,
        failure.value
          ? h("div", { class: "fbk-note", role: "alert", "data-tone": "error", "data-code": failure.value.code }, [
              h(StudioIcon, { name: "warning" }),
              h("span", null, failure.value.message),
              failure.value.code === FEEDBACK_CODE.unavailable
                ? h("button", { type: "button", class: "btn sm", "data-action": "feedback.link", onClick: () => props.onFile(FEEDBACK_REPO_ISSUES + "new/choose") }, t("feedback.go_to_github"))
                : null,
            ])
          : null,
        h("div", { class: "fbk-reply-foot" }, [
          limits
            ? h("button", {
                type: "button",
                class: "fbk-attach",
                "data-action": "feedback.shot.pick",
                disabled: sending.value || shots.value.length >= limits.images,
                title: t("feedback.you_can_also_paste_png", { n: limits.images, size: megabytes(limits.uploadBytes) }),
                onClick: () => picker.value?.click(),
              }, [h(StudioIcon, { name: "image" }), t("feedback.attach_image")])
            : null,
          limits
            ? h("input", {
                ref: picker,
                class: "sr-only",
                type: "file",
                accept: "image/png,image/jpeg",
                multiple: true,
                tabindex: -1,
                "aria-label": t("feedback.choose_screenshot_files"),
                "data-action": "feedback.shot.pick",
                onChange: (e: Event) => {
                  const input = e.target as HTMLInputElement;
                  void add([...(input.files ?? [])]);
                  input.value = "";
                },
              })
            : null,
          h("span", { id: `${id}-count`, class: "fbk-count", "data-over": over ? "" : undefined },
            t("feedback.n_max_bytes", { n: bytes.toLocaleString(), max: max.toLocaleString() })),
          h("button", { class: "fbk-send", type: "submit", disabled: blocked },
            [h(StudioIcon, { name: "send" }), t(sending.value ? "common.sending" : "common.send")]),
        ]),
      ]);
    };
  },
});
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import {
  FEEDBACK_CODE, FEEDBACK_REPO_ISSUES, canReplyTo, isUnderReview,
  type FeedbackItem, type FeedbackLimits, type FeedbackReply,
} from "../port/feedback";
import type { AgentPort } from "../port/port";
import { replyFailure, type FeedbackFailure } from "./feedbackfailure";
import FeedbackShot from "./FeedbackShot.vue";
import { admit, megabytes, payload, readShot, REFUSED_LABEL, type Refused, type Shot } from "./feedbackshots";
import StudioIcon from "./StudioIcon.vue";
import LevelBadge from "./LevelBadge.vue";
import { stamp } from "./feedbackstatus.vue";

const AUTHOR_LABEL = { maintainer: "feedback.maintainers", user: "chat.you" } as const;

const props = withDefaults(defineProps<{ item: FeedbackItem; level?: number | null; fresh: number }>(), { level: null });

// Replies are plain text by contract: they render as text nodes, never linked,
// formatted or interpreted, whoever wrote them.
const newIds = computed(() => new Set(props.fresh > 0 ? props.item.replies.filter((r) => r.author === "maintainer").slice(-props.fresh).map((r) => r.id) : []));
const isNew = (r: FeedbackReply) => newIds.value.has(r.id);
</script>

<template>
  <ol v-if="item.replies.length > 0" class="fbk-thread" :aria-label="t('feedback.conversation')">
    <li v-for="r in item.replies" :key="r.id" :data-author="r.author" :data-new="isNew(r) ? '' : undefined">
      <div class="fbk-msg-hd">
        <b>{{ t(AUTHOR_LABEL[r.author]) }}</b>
        <template v-if="r.author === 'user' && level !== null">
          <LevelBadge :level="level as number" decorative :size="14" />
          <span class="sr-only">{{ t("feedback.level_n", { n: level as number }) }}</span>
        </template>
        <span aria-hidden="true">·</span>
        <time :datetime="r.createdAt">{{ stamp(r.createdAt) }}</time>
        <span v-if="isNew(r)" class="fbk-new">{{ t("common.new") }}</span>
      </div>
      <p class="fbk-msg" dir="auto">{{ r.body }}</p>
    </li>
  </ol>
</template>
