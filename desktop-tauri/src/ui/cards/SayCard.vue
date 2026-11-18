<script lang="ts">
import type { LocalRefs } from "../Markdown.vue";

/** A quote on its way to the composer. n makes the same text twice two
 *  requests; turn is the kernel's name for the reply it came from, absent when
 *  the transcript was rebuilt without checkpoints. */
export interface Quote {
  text: string;
  turn?: number;
  // The workspace file the text was selected in, when it did not come from a reply.
  source?: string;
  n: number;
}

/** What a finished reply can be acted on with. Regeneration belongs to the
 *  individual reply and requires a paired checkpoint. */
export interface ReplyActions {
  files?: LocalRefs;
  activeFile?: string;
  onAddFile?: (reference: string) => void;
  openDeliveredFile?: (path: string) => Promise<void>;
  revealDeliveredFile?: (path: string) => Promise<void>;
  onQuote: (text: string, id: string) => void;
  onQuoteFile?: (text: string, source: string) => void;
  canRegenerate: (id: string) => boolean;
  hasLaterTurns: (id: string) => boolean;
  onRegenerate: (id: string) => void;
  canBranch: (id: string) => boolean;
  onBranch: (id: string) => void | Promise<void>;
  model?: string;
  onConfigureModel?: () => void;
  onRunDetail?: () => void;
}
</script>

<script setup lang="ts">
import { computed, defineComponent, h, nextTick, onMounted, onScopeDispose, ref, watch, type PropType, type VNodeChild } from "vue";
import { t } from "../../i18n";
import { count, decimals } from "../../i18n/format";
import type { Item } from "../../state/session";
import { thoughtStartedAt } from "../../state/say";
import { useFoldModes, useStartsOpen, useThinkingPreview } from "../../state/foldpref";
import { useThoughtFold } from "../thoughtfold";
import { useRevealed } from "../reveal";
import { useShown } from "../shown";
import { useDismiss } from "../dismiss";
import { pinToViewport } from "../place";
import SelectionBar, { copySelection } from "../SelectionBar.vue";
import { useSelectionBar } from "../useSelectionBar";
import StudioIcon from "../StudioIcon.vue";
import Sym from "../Sym.vue";
import LazyMarkdown from "../LazyMarkdown.vue";
import Boundary from "../Boundary.vue";
import CopyButton from "../CopyButton.vue";
import TooltipButton from "../TooltipButton.vue";
import ReplyMenu from "./ReplyMenu.vue";

type Say = Extract<Item, { t: "say" }>;

// Folded, the only thing left of a thought is how much of the turn it was. The
// spec puts both halves there — how long, and how much — because either alone
// hides whether a slow turn was spent thinking or waiting.
const ThoughtLabel = defineComponent({
  name: "ThoughtLabel",
  props: { item: { type: Object as PropType<Say>, required: true } },
  setup(props) {
    return () => {
      // Graphemes, not code units: an emoji in the reasoning is one character
      // to the reader and two to the string.
      const graphemes = [...(props.item.reasoning ?? "")].length;
      const chars = t("chat.n_chars", { n: count(graphemes) });
      const secs = decimals((props.item.thoughtMs ?? 0) / 1000, 1);
      if (props.item.cut) return t(props.item.cut === "stopped" ? "chat.thought_stopped_s" : "chat.thought_failed_s", { secs });
      return props.item.thoughtMs ? t("chat.thought_chars_secs_s", { secs, chars }) : t("chat.thought_chars", { chars });
    };
  },
});

const LIVE_TICK_MS = 100;

// While the model is still thinking the reader sees the clock run, not a bare
// ellipsis; a card this window did not watch begin has no start to count from.
const LiveThought = defineComponent({
  name: "LiveThought",
  props: { id: { type: String, required: true } },
  setup(props) {
    const since = thoughtStartedAt(props.id);
    const watched = useShown();
    const now = ref(Date.now());
    let timer: ReturnType<typeof setInterval> | undefined;
    onMounted(() => {
      if (since === undefined || !watched) return;
      now.value = Date.now();
      timer = setInterval(() => (now.value = Date.now()), LIVE_TICK_MS);
    });
    onScopeDispose(() => clearInterval(timer));
    return () => since === undefined
      ? t("chat.thinking")
      : t("chat.thinking_secs_s", { secs: decimals(Math.max(0, now.value - since) / 1000, 1) });
  },
});

// Content the transcript draws under one answer without making the card own it.
const AfterAnswer = defineComponent({
  name: "AfterAnswer",
  props: { node: { type: null as unknown as PropType<VNodeChild>, default: undefined } },
  setup(props) {
    return () => props.node ?? null;
  },
});

// A ref is "provider/model"; the model is what identifies it to a reader, and
// the provider stays in the title for when two of them carry the same name.
const modelName = (ref: string) => ref.slice(ref.lastIndexOf("/") + 1);

// The browser keeps selections outside the answer, including across cards.
function selectedIn(card: Element | null): string {
  const sel = card && window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount !== 1) return "";
  const range = sel.getRangeAt(0);
  if (!card.contains(range.commonAncestorContainer)) return "";
  return sel.toString().trim();
}

function download(text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "text/markdown;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = "reasonix-reply.md";
  link.click();
  URL.revokeObjectURL(url);
}

const props = withDefaults(defineProps<{
  item: Say;
  afterAnswer?: VNodeChild;
  settled?: boolean;
  reply?: ReplyActions;
  quiet?: boolean;
}>(), { settled: true });

const thinking = computed(() => !props.item.done && props.item.thoughtMs === undefined);
const start = useStartsOpen("thinking", () => thinking.value && !props.item.text);
const modes = useFoldModes();
const alwaysOpen = computed(() => modes.value.thinking === "open");
const folds = useThoughtFold();
const touched = ref<boolean | null>(folds.get(props.item.id) ?? null);
const open = computed(() => touched.value ?? start.value);
const showsPreview = useThinkingPreview();
const peeks = computed(() => showsPreview.value && !open.value);
const peek = computed(() => (peeks.value ? (props.item.reasoning ?? "").trimStart() : ""));

// The title changes once, when a thought this card watched finishes; a
// thought drawn already finished is not that change happening.
const settledHere = ref(false);
let settleTimer: ReturnType<typeof setTimeout> | undefined;
watch(thinking, (now, before) => {
  if (!(before && !now)) return;
  settledHere.value = true;
  clearTimeout(settleTimer);
  settleTimer = setTimeout(() => (settledHere.value = false), 240);
});
onScopeDispose(() => clearTimeout(settleTimer));

const menu = ref<"" | "retry" | "more">("");
const shut = () => (menu.value = "");
const toggleMenu = (which: "retry" | "more") => (menu.value = menu.value === which ? "" : which);
const branching = ref(false);

const retry = ref<InstanceType<typeof TooltipButton> | null>(null);
const more = ref<InstanceType<typeof TooltipButton> | null>(null);
const answer = ref<HTMLDivElement | null>(null);
const selectionMenu = ref<HTMLDivElement | null>(null);
const quoteItem = ref<HTMLButtonElement | null>(null);
const selection = ref<{ text: string; x: number; y: number } | null>(null);

const closeSelection = () => {
  if (selectionMenu.value?.contains(document.activeElement)) answer.value?.focus();
  selection.value = null;
};
useDismiss(computed(() => !!selection.value), selectionMenu, closeSelection);

const { at: barAt, settle: settleBar, clear: clearBar } = useSelectionBar(answer, selectedIn);

const offerSelection = (x: number, y: number) => {
  if (!props.item.done || !props.reply) return false;
  const text = selectedIn(answer.value);
  if (!text) return false;
  if (x === 0 && y === 0) {
    const rect = window.getSelection()!.getRangeAt(0).getBoundingClientRect();
    x = rect.left;
    y = rect.bottom;
  }
  menu.value = "";
  clearBar();
  selection.value = { text, x, y };
  return true;
};

watch(selection, async (sel) => {
  if (!sel) return;
  await nextTick();
  if (selectionMenu.value) pinToViewport(selectionMenu.value, sel.x, sel.y);
  quoteItem.value?.focus();
});

// Thinking is the longest-running stream of the turn — 10s of it before the
// first answer token, measured — so it gets the same paced reveal the answer
// does rather than tracking the wire's bursts.
const thought = useRevealed(() => props.item.reasoning ?? "", () => !props.item.done && open.value);

const fallback = computed(() => h("div", { class: "srcfb" }, [
  h("div", { class: "srcfb-hd" }, [
    h("span", { class: "srcfb-tag" }, t("chat.shown_as_written")),
    h("span", null, t("chat.this_layout_did_not_show")),
  ]),
  h("div", { class: "md srcfb-src" }, props.item.text),
]));

const onToggle = (e: Event) => {
  const next = (e.currentTarget as HTMLDetailsElement).open;
  if (next === open.value) return;
  touched.value = next;
  folds.set(props.item.id, next);
};

const onMouseUp = () => {
  if (props.item.done && props.reply) settleBar();
};

const onContextMenu = (e: MouseEvent) => {
  if (!offerSelection(e.clientX, e.clientY)) return;
  e.preventDefault();
  e.stopPropagation();
};

const onKeyDown = (e: KeyboardEvent) => {
  if (e.key !== "ContextMenu" && !(e.key === "F10" && e.shiftKey)) return;
  if (!offerSelection(0, 0)) return;
  e.preventDefault();
  e.stopPropagation();
};

const walkSelectionMenu = (e: KeyboardEvent) => {
  const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
  if (!keys.includes(e.key)) return;
  const all = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]')];
  const at = all.indexOf(document.activeElement as HTMLElement);
  const next = e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
  e.preventDefault();
  all[next]?.focus();
};

const quoteAnswer = () => {
  if (!props.reply) return;
  props.reply.onQuote(selectedIn(answer.value) || props.item.text, props.item.id);
};

const branch = () => {
  if (!props.reply) return;
  branching.value = true;
  void Promise.resolve(props.reply.onBranch(props.item.id)).finally(() => (branching.value = false));
};

const retryNow = () => {
  menu.value = "";
  props.reply?.onRegenerate(props.item.id);
};
const configureModel = () => {
  menu.value = "";
  props.reply?.onConfigureModel?.();
};
const downloadReply = () => {
  menu.value = "";
  download(props.item.text);
};
const runDetail = () => {
  menu.value = "";
  props.reply?.onRunDetail?.();
};
const quoteBar = () => {
  if (!barAt.value || !props.reply) return;
  props.reply.onQuote(barAt.value.text, props.item.id);
  clearBar();
};

const quoteSelection = () => {
  const sel = selection.value;
  if (!sel || !props.reply) return;
  selection.value = null;
  props.reply.onQuote(sel.text, props.item.id);
};
</script>

<template>
  <div class="call" data-k="say" :data-quiet="quiet ? '' : undefined">
    <div class="g">
      <Sym icon="agent" />
    </div>
    <div class="c">
      <div class="hl">
        <span class="nm">reasonix</span>
        <!-- Which model wrote this one. A turn can carry two, and without it
             both read as whatever the composer happens to name now. -->
        <span v-if="item.model" class="src" :title="item.model">{{ modelName(item.model) }}</span>
      </div>
      <div class="out">
        <details v-if="item.reasoning?.trim()" class="think" :open="open" :data-live="thinking ? '' : undefined"
          :data-full="alwaysOpen ? '' : undefined" :data-cut="item.cut" :data-settled="settledHere ? '' : undefined"
          @toggle="onToggle">
          <summary>
            <span class="fold">
              <span class="think-swap">
                <span v-if="settledHere" class="think-label think-out" aria-hidden="true">
                  {{ t("chat.thinking_secs_s", { secs: decimals((item.thoughtMs ?? 0) / 1000, 1) }) }}
                </span>
                <span :key="thinking ? 'live' : 'done'" :class="thinking ? 'think-label think-sweep' : 'think-label'">
                  <LiveThought v-if="thinking" :id="item.id" />
                  <ThoughtLabel v-else :item="item" />
                </span>
              </span>
            </span>
            <span v-if="item.cut && !open" class="think-note">{{ t(item.cut === "stopped" ? "chat.thought_stopped_note" : "chat.thought_failed_note") }}</span>
            <span v-if="peek && !item.cut" class="thinking-preview">{{ peek }}</span>
          </summary>
          <div class="tk">
            <div class="tk-in">
              {{ thought }}
              <span v-if="thinking" class="caret" />
            </div>
          </div>
        </details>
        <div v-if="item.text" ref="answer" class="txt" :tabindex="item.done && reply ? 0 : undefined"
          data-action-contextmenu="reply.selection.menu" data-action-keydown="reply.selection.menu"
          data-action-mouseup="reply.selection.bar" :data-target="item.id"
          @mouseup="onMouseUp" @contextmenu="onContextMenu" @keydown="onKeyDown">
          <Boundary :retry-key="item.text" :fallback="fallback">
            <LazyMarkdown :text="item.text" :streaming="!item.done" :local="reply?.files" />
          </Boundary>
        </div>
        <AfterAnswer :node="afterAnswer" />
        <!-- Only once the answer is whole: copying half a stream hands over
             something that was never said, and a row of actions under work
             that is still running reads as the turn being over. -->
        <div v-if="item.done && settled && item.text.trim()" class="acts">
          <CopyButton :text="item.text" icon-only />
          <TooltipButton v-if="reply" type="button" data-action="reply.quote" :data-target="item.id"
            :title="t('chat.quote_into_the_composer')" :aria-label="t('chat.quote_into_the_composer')" @click="quoteAnswer">
            <StudioIcon name="quote" />
          </TooltipButton>
          <TooltipButton v-if="reply?.canBranch(item.id)" type="button" data-action="reply.branch"
            :title="t('chat.create_a_conversation_branch')" :aria-label="t('chat.create_a_conversation_branch')" :disabled="branching" @click="branch">
            <StudioIcon :name="branching ? 'clock' : 'fork'" />
          </TooltipButton>
          <span v-if="reply?.canRegenerate(item.id)" class="acts-menu">
            <TooltipButton ref="retry" type="button" data-action="reply.retry" :title="t('chat.regenerate')"
              :aria-label="t('chat.regenerate')" :aria-expanded="menu === 'retry'" @click="toggleMenu('retry')">
              <StudioIcon name="refresh" />
            </TooltipButton>
            <ReplyMenu :anchor="retry?.anchor ?? null" :open="menu === 'retry'" :on-close="shut">
              <div class="acts-pop-head">
                {{ t("chat.regenerate") }}
                <small>{{ t("chat.the_current_reply_stays_in") }}</small>
                <small v-if="reply?.hasLaterTurns(item.id)">{{ t("chat.everything_after_this_turn_is") }}</small>
              </div>
              <button type="button" role="menuitem" data-action="reply.retry-now" @click="retryNow">
                <StudioIcon name="refresh" /><span>{{ t("chat.retry_as_configured") }}</span><small v-if="reply?.model">{{ reply.model }}</small>
              </button>
              <button v-if="reply?.onConfigureModel" type="button" role="menuitem" data-action="reply.configure" @click="configureModel">
                <StudioIcon name="sliders" /><span>{{ t("chat.adjust_the_model_and_effort") }}</span>
              </button>
            </ReplyMenu>
          </span>
          <span v-if="reply" class="acts-menu">
            <TooltipButton ref="more" type="button" data-action="reply.more" :title="t('common.more')"
              :aria-label="t('common.more')" :aria-expanded="menu === 'more'" @click="toggleMenu('more')">
              <StudioIcon name="more" />
            </TooltipButton>
            <ReplyMenu :anchor="more?.anchor ?? null" :open="menu === 'more'" :on-close="shut">
              <div class="acts-pop-head">{{ t("chat.this_reply") }}</div>
              <button type="button" role="menuitem" data-action="reply.download" @click="downloadReply">
                <StudioIcon name="download" /><span>{{ t("chat.download_the_reply") }}</span><small>Markdown</small>
              </button>
              <template v-if="reply?.onRunDetail">
                <div class="acts-pop-head">{{ t("chat.this_turn") }}</div>
                <button type="button" role="menuitem" data-action="reply.run-detail" @click="runDetail">
                  <StudioIcon name="activity" /><span>{{ t("common.run_analysis") }}</span>
                </button>
              </template>
            </ReplyMenu>
          </span>
        </div>
      </div>
    </div>
    <SelectionBar v-if="barAt && reply && !selection" :at="barAt">
      <button type="button" data-action-click="reply.quote" data-action-mousedown="reply.quote" :data-target="item.id"
        @mousedown.prevent @click="quoteBar">
        <StudioIcon name="quote" /><span>{{ t("common.quote_selected_text") }}</span>
      </button>
      <button type="button" data-action-click="reply.selection.copy" data-action-mousedown="reply.selection.copy" :data-target="item.id"
        @mousedown.prevent @click="copySelection(barAt.text); clearBar()">
        <StudioIcon name="copy" /><span>{{ t("chat.copy_selected_text") }}</span>
      </button>
    </SelectionBar>
    <Teleport to="body">
      <div v-if="selection && reply" ref="selectionMenu" class="acts-pop" role="menu" :aria-label="t('chat.selected_text')"
        data-action-keydown="reply.selection.menu" :data-target="item.id" @keydown="walkSelectionMenu">
        <button ref="quoteItem" type="button" role="menuitem" data-action="reply.quote" :data-target="item.id" @click="quoteSelection">
          <span>{{ t("common.quote_selected_text") }}</span>
        </button>
        <CopyButton :text="selection.text" role="menuitem" :label="t('chat.copy_selected_text')" />
      </div>
    </Teleport>
  </div>
</template>
