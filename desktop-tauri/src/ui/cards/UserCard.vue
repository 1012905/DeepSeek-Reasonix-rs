<script setup lang="ts">
import { computed, defineComponent, h, nextTick, ref, watch } from "vue";
import type { AgentPort, Checkpoint, RewindPlan, RewindResult, RewindScope } from "../../port/port";
import type { Item } from "../../state/session";
import RewindControl from "./RewindControl.vue";
import SkillPinView from "./SkillPinView.vue";
import { parseSkillPin } from "./skillpin";
import CopyButton from "../CopyButton.vue";
import { reason } from "../../i18n/kernel";
import { t } from "../../i18n";
import { clock } from "../../i18n/format";
import StudioIcon from "../StudioIcon.vue";
import { messageSource } from "../source";
import { useSendShortcut } from "../../state/prefs";
import { sendAria, sendHint, sendsOnKey } from "../sendkey";
import { useIme } from "../ime";
import { touchKeyboard } from "../touchKeyboard";
import { useFitHeight } from "../fitHeight";
import { useViewer } from "../../state/viewer";
import ImagePreview from "../ImagePreview.vue";

const props = withDefaults(defineProps<{
  item: Extract<Item, { t: "user" }>;
  port?: Pick<AgentPort, "workspaceImageURL">;
  cp?: Checkpoint;
  onResend?: (turn: number, text: string) => Promise<void>;
  onWithdraw?: (itemId: string) => void | Promise<void>;
  onPrepareRewind?: (turn: number, scope: RewindScope) => Promise<RewindPlan>;
  onCommitRewind?: (planId: string, text?: string) => Promise<RewindResult>;
}>(), {
  port: undefined, cp: undefined, onResend: undefined, onWithdraw: undefined,
  onPrepareRewind: undefined, onCommitRewind: undefined,
});

const SavedImage = defineComponent({
  name: "SavedImage",
  props: { path: { type: String, required: true }, src: { type: String, required: true } },
  setup(p) {
    const failed = ref(false);
    const name = p.path.split("/").at(-1) ?? p.path;
    return () => h("div", { class: "user-image" }, failed.value
      ? h("span", null, t("chat.image_unavailable"))
      : h(ImagePreview, { src: p.src, name, className: "user-image-trigger" }, () => h("img", {
        src: p.src, alt: name, loading: "lazy", width: 240, height: 160,
        onError: () => (failed.value = true),
      })));
  },
});

// A rewind needs a turn the kernel claimed, and a queued line has not happened
// yet — there is nothing behind it to take back.
const shortcut = useSendShortcut();
const ime = useIme();
const viewer = useViewer();
const editable = computed(() => !!props.onResend && !!props.cp && !props.item.pending);
const draft = ref<string | null>(null);
const sending = ref(false);
const failed = ref("");
const box = ref<HTMLTextAreaElement | null>(null);
const source = computed(() => messageSource(props.item.via, viewer.value));
const reopen = computed(() => editable.value && draft.value === null);
const rewind = computed(() => !!(props.cp && props.onPrepareRewind && props.onCommitRewind));
const pin = computed(() => parseSkillPin(props.item.text));
const images = computed(() => [...new Set((props.item.attachments ?? []).map((a) => a.path))]);

watch(() => draft.value === null, async (isNull) => {
  if (isNull) return;
  await nextTick();
  const el = box.value;
  if (!el) return;
  el.focus();
  el.setSelectionRange(el.value.length, el.value.length);
}, { flush: "post" });

useFitHeight(box, () => draft.value ?? "");

const resend = () => {
  const text = (draft.value ?? "").trim();
  const cp = props.cp;
  const onResend = props.onResend;
  if (!editable.value || !text || sending.value || !cp || !onResend) return;
  sending.value = true;
  failed.value = "";
  onResend(cp.turn, text)
    .then(() => (draft.value = null))
    .catch((e: unknown) => (failed.value = reason(e)))
    .finally(() => (sending.value = false));
};

const edit = (event: Event) => {
  draft.value = (event.target as HTMLTextAreaElement).value;
};
const onKeyDown = (ev: KeyboardEvent) => {
  if (ime.isIme(ev)) {
    if (ev.key === "Enter") ev.preventDefault();
    if (ev.key === "Escape") ev.stopPropagation();
    return;
  }
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    draft.value = null;
  } else if (sendsOnKey(ev, shortcut.value, touchKeyboard())) {
    ev.preventDefault();
    resend();
  }
};
const withdraw = () => {
  if (props.item.itemId && props.onWithdraw) void props.onWithdraw(props.item.itemId);
};
const prepareRewind = (turn: number, scope: RewindScope) => props.onPrepareRewind!(turn, scope);
const commitRewind = (planId: string) => props.onCommitRewind!(planId, props.item.text);
</script>

<template>
  <div v-if="item.pending && item.queued === 'steer' && item.itemId && onWithdraw" class="steerwait" data-pending="">
    <div class="sw-hd">
      {{ t("composer.steer_waiting") }}
      <button type="button" class="sw-undo" data-action="steer.withdraw" :data-target="item.itemId" @click="withdraw">
        {{ t("composer.withdraw") }}
      </button>
    </div>
    <div class="sw-txt" dir="auto">{{ item.text }}</div>
  </div>
  <div v-else class="call" data-k="me" :data-pending="item.pending ? '' : undefined">
    <div class="g">
      <span class="sym"><StudioIcon name="user" /><span class="sr-only">{{ t("chat.you") }}</span></span>
    </div>
    <div class="c">
      <div v-if="item.steer || source" class="hl user-hl">
        <!-- It reached the model inside a turn already running, which is why
             there is no checkpoint on this row to rewind to. -->
        <span v-if="item.steer" class="steermark">{{ t("common.steer") }}</span>
        <span v-if="source" class="viamark">{{ source }}</span>
      </div>
      <div class="out">
        <template v-if="draft === null">
          <div v-if="port && images.length > 0" class="user-images">
            <SavedImage v-for="path in images" :key="port.workspaceImageURL(path)" :path="path" :src="port.workspaceImageURL(path)" />
          </div>
          <SkillPinView v-if="pin" :pin="pin" />
          <div v-else class="txt" dir="auto">{{ item.text }}</div>
        </template>
        <div v-else class="reask">
          <textarea
            ref="box"
            dir="auto"
            data-action-change="turn.edit"
            data-action-keydown="turn.resend"
            :data-target="item.id"
            :value="draft"
            :rows="1"
            :readonly="sending"
            :aria-keyshortcuts="sendAria(shortcut, touchKeyboard())"
            :title="sendHint(shortcut, false, touchKeyboard())"
            :aria-label="t('chat.rewrite_this_message')"
            @input="edit"
            @keydown="onKeyDown"
            @compositionstart="ime.handlers.onCompositionStart"
            @compositionend="ime.handlers.onCompositionEnd"
          />
          <div v-if="failed" class="txt bad">{{ failed }}</div>
          <div class="reask-ft">
            <button class="btn" data-action="turn.resend" :data-target="item.id" :disabled="sending || !(draft ?? '').trim()" @click="resend">
              {{ sending ? t("chat.sending") : t("chat.send_the_edit") }}
            </button>
            <button class="dismiss" data-action="turn.edit" data-value="cancel" @click="draft = null">
              {{ t("common.cancel") }}
            </button>
            <span class="hint">{{ t("chat.everything_after_this_turn_is") }}</span>
          </div>
        </div>
      </div>
      <div class="user-acts">
        <time v-if="item.at" class="user-time" :datetime="new Date(item.at).toISOString()" :title="new Date(item.at).toLocaleString()">{{ clock(item.at) }}</time>
        <CopyButton :text="item.text" icon-only show-feedback :label="t('common.copy')" />
        <button
          v-if="reopen"
          type="button"
          class="reask-open"
          data-action="turn.edit"
          :data-target="item.id"
          :title="t('chat.rewrite_this_message_and_send')"
          :aria-label="t('chat.rewrite')"
          @click="draft = item.text"
        >
          <StudioIcon name="edit" />
        </button>
        <RewindControl v-if="rewind && cp" :cp="cp" compact :on-prepare="prepareRewind" :on-commit="commitRewind" />
      </div>
    </div>
  </div>
</template>
