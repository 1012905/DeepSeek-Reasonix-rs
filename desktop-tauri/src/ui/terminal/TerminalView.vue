<script lang="ts">
import type { AgentPort } from "../../port/port";

export interface TerminalViewProps {
  port: AgentPort;
  onError: (e: unknown) => void;
  /** Whether this view is on screen. A hidden view keeps its screens' last frame
   *  and holds no connection to the kernel. */
  visible: boolean;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";
import { useTerminalTabs } from "../useTerminalTabs";
import WorkbenchState from "../WorkbenchState.vue";
import WorkbenchTerminals from "./WorkbenchTerminals.vue";

const props = defineProps<TerminalViewProps>();

const selected = ref("");
// The terminals this view closed: a dismissed key is drawn by no view at all,
// so it is held here rather than read back from the list on every poll.
const dismissed = ref<Set<string>>(new Set());
// Nothing outside this view asks for a terminal, so the request counter stands still.
const request = ref(0);

const hook = useTerminalTabs({
  port: () => props.port,
  shown: () => props.visible,
  pick: (key) => { selected.value = key; },
  setDismissed: (next) => {
    dismissed.value = typeof next === "function" ? next(dismissed.value) : next;
  },
  // A link clicked in a terminal leaves for the system browser; the hook itself
  // decides whether this shell draws a page instead.
  onExternal: (url) => void props.port.openExternal(url).catch((e: unknown) => props.onError(e)),
  onError: (message) => props.onError(new Error(message)),
  request,
});

const tabs = computed(() => hook.tabs.value.filter((tab) => !dismissed.value.has(`terminal:${tab.id}`)));
const shown = computed(() => hook.terminals.value.filter((info) => !dismissed.value.has(`terminal:${info.id}`)));

// The tab in front follows the reader's pick, and falls back to the newest one
// when that tab is gone.
const active = computed(() => {
  const keys = tabs.value.map((tab) => `terminal:${tab.id}`);
  if (keys.includes(selected.value)) return selected.value.slice("terminal:".length);
  return tabs.value.at(-1)?.id ?? "";
});

const scheme = computed<"light" | "dark">(() =>
  document.documentElement.dataset.theme === "light" ? "light" : "dark",
);

// Ending a shell ends whatever runs in it, so a busy one asks first; the hook
// owns the ask, the close itself is this view's.
const close = (tabIds: string[]) => {
  const targets = tabs.value.filter((tab) => tabIds.includes(tab.id));
  void hook.guard(targets, async (surfaces) => {
    for (const surface of surfaces) {
      if (surface.kind !== "terminal") continue;
      try {
        await props.port.closeTerminal(surface.id);
      } catch (e) {
        props.onError(e);
      }
      dismissed.value = new Set([...dismissed.value, `terminal:${surface.id}`]);
    }
  });
};
</script>

<template>
  <section class="terminal-view" :aria-label="t('terminal.name')" data-shell="terminal">
    <header class="workbench-tabs">
      <div class="workbench-tablist" role="tablist" :aria-label="t('terminal.name')">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          class="workbench-tab"
          role="tab"
          :aria-selected="active === tab.id"
          :data-key="`terminal:${tab.id}`"
        >
          <button
            class="workbench-tab-pick"
            type="button"
            data-action-click="terminal.select"
            :data-target="`terminal:${tab.id}`"
            :title="tab.name"
            @click="selected = `terminal:${tab.id}`"
          >
            <StudioIcon name="terminal" />
            <span>{{ tab.name }}</span>
            <i v-if="tab.busy" class="workbench-tab-busy" aria-hidden="true" />
          </button>
          <button
            class="workbench-tab-close"
            type="button"
            data-action="terminal.close"
            :data-target="`terminal:${tab.id}`"
            :aria-label="t('common.disable_name', { name: tab.name })"
            :title="t('common.close')"
            @click="close([tab.id])"
          >
            <StudioIcon name="close" />
          </button>
        </div>
      </div>
      <TooltipButton
        class="workbench-new"
        data-action="workbench.launch-terminal"
        :title="t('terminal.open')"
        :aria-label="t('terminal.open')"
        @click="void hook.open()"
      >
        <StudioIcon name="plus" />
      </TooltipButton>
    </header>
    <div class="terminal-body">
      <WorkbenchState
        v-if="tabs.length === 0"
        icon="terminal"
        :title="t('terminal.open')"
        :detail="t('terminal.launch_hint')"
      >
        <button type="button" data-action="workbench.launch-terminal" @click="void hook.open()">{{ t('terminal.open') }}</button>
      </WorkbenchState>
      <WorkbenchTerminals
        v-else
        :port="port"
        :terminals="shown"
        :active="active"
        :visible="visible"
        :scheme="scheme"
        :shells="hook.shells.value"
        :on-load-shells="hook.loadShells"
        :on-shell="(kind: string) => void hook.open(kind)"
        :on-open-page="(url: string) => void hook.openUrl(url)"
        :on-command="hook.submitted"
        :on-restart="hook.restart"
        :on-close="(id: string) => close([id])"
        :on-error="onError"
        :asking="hook.asking.value"
        :on-cancel-ask="hook.cancelAsk"
        :on-confirm-ask="hook.confirmAsk"
      />
    </div>
  </section>
</template>

<style scoped>
/* The shell the panel's own classes do not cover: this view is a column of its
   own, outside the workbench's tab strip. */
.terminal-view { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--raised); }
.terminal-body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
</style>
