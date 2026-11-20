<script lang="ts">
export type FeedbackTab = "send" | "mine";
</script>

<script setup lang="ts">
import { onMounted, onUnmounted, ref, shallowRef } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import { useEscape } from "./dismiss";
import { listenAction } from "./listen";
import FeedbackForm from "./FeedbackForm.vue";
import FeedbackMine from "./FeedbackMine.vue";
import StudioIcon from "./StudioIcon.vue";

const TABS: readonly FeedbackTab[] = ["send", "mine"];
const TAB_LABEL: Record<FeedbackTab, string> = { send: "feedback.write_new", mine: "session.my_feedback" };
const FOCUSABLE = 'a[href], button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])';

const props = withDefaults(defineProps<{
  port: AgentPort;
  tab?: FeedbackTab;
  onClose: () => void;
  onError: (message: string) => void;
  onUnread?: (n: number) => void;
  // Reports waiting on the reader, counted on the My feedback tab.
  unread?: number;
}>(), { tab: "send", unread: 0 });

const tab = ref<FeedbackTab>(props.tab);
const slot = ref<HTMLDivElement | null>(null);
const concealed = ref(false);
const card = ref<HTMLDivElement | null>(null);
const opener = document.activeElement;
// FeedbackForm reads the box through the ref, so it receives the ref itself.
const pasteRoot = shallowRef(card);
useEscape(true, () => props.onClose());

onMounted(() => {
  card.value?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus();
});
onUnmounted(() => {
  if (opener instanceof HTMLElement) opener.focus();
});

onMounted(() => {
  const el = card.value;
  if (!el) return;
  const trap = (e: Event) => {
    const ev = e as globalThis.KeyboardEvent;
    if (ev.key !== "Tab") return;
    const all = [...el.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((n) => n.offsetParent !== null || n === document.activeElement);
    const head = all[0];
    const tail = all[all.length - 1];
    if (!head || !tail) return;
    const inside = el.contains(document.activeElement);
    if (ev.shiftKey && (document.activeElement === head || !inside)) {
      ev.preventDefault();
      tail.focus();
    } else if (!ev.shiftKey && (document.activeElement === tail || !inside)) {
      ev.preventDefault();
      head.focus();
    }
  };
  const off = listenAction(el, "keydown", { action: "feedback.focus", listener: trap });
  onUnmounted(off);
});

const openLink = (url: string) => void props.port.openExternal(url).catch((e) => props.onError(reason(e)));

const walk = (e: KeyboardEvent, at: FeedbackTab) => {
  const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
  if (!step) return;
  e.preventDefault();
  const next = TABS[(TABS.indexOf(at) + step + TABS.length) % TABS.length]!;
  tab.value = next;
  requestAnimationFrame(() => card.value?.querySelector<HTMLElement>(`#fbk-tab-${next}`)?.focus());
};
</script>

<template>
  <div class="fbk-veil" :hidden="concealed">
    <div ref="card" class="fbk" role="dialog" aria-modal="true" aria-labelledby="fbk-title" :data-tab="tab">
      <header class="fbk-hd">
        <h2 id="fbk-title">{{ t("feedback.feedback") }}</h2>
        <div class="fbk-tabs" role="tablist" :aria-label="t('feedback.feedback')">
          <button
            v-for="id in TABS"
            :id="`fbk-tab-${id}`"
            :key="id"
            role="tab"
            type="button"
            :aria-selected="tab === id"
            :aria-controls="`fbk-panel-${id}`"
            :tabindex="tab === id ? 0 : -1"
            data-action-click="feedback.tab"
            data-action-keydown="feedback.tab"
            :data-value="id"
            @click="tab = id"
            @keydown="walk($event, id)"
          >
            {{ t(TAB_LABEL[id]) }}
            <span v-if="id === 'mine' && unread > 0" class="fbk-tab-count" aria-hidden="true">{{ unread }}</span>
          </button>
        </div>
        <div ref="slot" class="fbk-hd-slot" :hidden="tab !== 'mine'" />
        <button class="fbk-icon" type="button" data-action="feedback.close" :aria-label="t('common.close')" :title="t('common.close')" @click="onClose()">
          <StudioIcon name="close" />
        </button>
      </header>
      <div class="fbk-body">
        <div id="fbk-panel-send" role="tabpanel" aria-labelledby="fbk-tab-send" :hidden="tab !== 'send'">
          <FeedbackForm :port="port" :paste-root="pasteRoot" :active="tab === 'send' && !concealed" :on-mine="() => (tab = 'mine')" :on-close="onClose" :on-file="openLink" :on-conceal="(v: boolean) => (concealed = v)" />
        </div>
        <div v-if="tab === 'mine'" id="fbk-panel-mine" role="tabpanel" aria-labelledby="fbk-tab-mine">
          <FeedbackMine :port="port" :on-file="openLink" :on-unread="onUnread" :slot="slot" />
        </div>
      </div>
    </div>
  </div>
</template>