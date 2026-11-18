<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { host } from "../port/host";
import type { HubPort } from "../port/hub";
import type { AgentPort, SessionStatus, WorkspaceInfo } from "../port/port";
import type { PaneView } from "../state/paneview";
import { useReachable, useViewer } from "../state/viewer";
import { useFolded } from "./usefold";
import { chord, ariaChord } from "./keys";
import DeviceBar from "./chrome/DeviceBar.vue";
import PhonePop from "./chrome/PhonePop.vue";
import TooltipButton from "./TooltipButton.vue";
import StudioIcon from "./StudioIcon.vue";
import ViewSwitch from "./chrome/ViewSwitch.vue";
import WindowControls from "./chrome/WindowControls.vue";

const props = defineProps<{
  /** Set when the focused pane is driven from another machine, naming it. */
  remoteHost?: string;
  /** Null while the window has no pane: the chrome still draws, and the few
   *  controls that need a session simply do nothing until one is focused. */
  port: AgentPort | null;
  status: SessionStatus | null;
  workspace: WorkspaceInfo | null;
  title?: string;
  steer: number;
  theme: string;
  view: PaneView;
  rows: number;
  surfaces: number;
  onTheme: () => void;
  onFind: () => void;
  onView: (to: PaneView) => void;
  onOpenProject?: (root: string) => void;
  onError?: (e: unknown) => void;
  hasHub?: boolean;
  hub?: HubPort | null;
}>();

const root = computed(() => props.status?.workspaceRoot || props.status?.cwd || "");
const project = computed(() => {
  const p = root.value;
  if (!p) return "—";
  return p.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || p;
});

const phone = useFolded("phone");
const gathered = useFolded("bar") || phone.value;
const device = useViewer();
const reachable = useReachable();
const more = ref(false);

const canOpenProject = computed(() => !!props.hub && root.value !== "" && !props.remoteHost && host().opensFolders());

function openProject() {
  if (props.hub && root.value) props.onOpenProject?.(root.value);
}

function sessionName(title?: string, p?: string): string {
  const trimmed = title?.trim();
  if (trimmed) return trimmed;
  if (p) {
    const base = p.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || p;
    return base.replace(/\.jsonl$/, "");
  }
  return t("common.new_session");
}

function zoomOnTitleBar(e: MouseEvent) {
  const d = document.documentElement.dataset;
  if (!(d.titlebar === "app" && (d.platform === "darwin" || d.platform === "windows"))) return;
  const el = e.target as HTMLElement | null;
  if (el?.closest("button, input, textarea, .picker, .menu")) return;
  host().toggleMaximiseWindow();
}
</script>

<template>
  <div>
    <DeviceBar v-if="hasHub && hub" :hub="hub" />
    <div class="chrome" @dblclick="zoomOnTitleBar">
      <div class="crumb">
        <span class="crumb-context">
          <button
            type="button"
            class="crumb-folder"
            data-action="chrome.open-project"
            :aria-label="t('app.open_project_folder')"
            :title="t('app.open_project_folder')"
            :disabled="!canOpenProject"
            @click="openProject()"
          >
            <StudioIcon name="folder" />
          </button>
          <span class="crumb-proj" :title="root">{{ project }}</span>
          <span v-if="workspace?.isolated && workspace.current === root" class="isolab" :title="root">
            {{ t("common.isolated_worktree") }}
          </span>
          <span class="isolab hostlab" :hidden="!remoteHost">{{ remoteHost }}</span>
          <span class="crumbsep">/</span>
        </span>
        <b :title="status?.sessionPath">{{ sessionName(title, status?.sessionPath) }}</b>
        <span v-if="phone && device" class="crumb-sub" :data-state="!reachable ? 'down' : status?.running ? 'run' : 'ok'">
          <i aria-hidden="true" />
          {{ !reachable
            ? t("remote.machine_disconnected", { machine: device.machine })
            : status?.running
              ? t("remote.running_on_machine", { machine: device.machine })
              : `${device.machine} · ${project}` }}
        </span>
      </div>

      <span class="badge" :hidden="steer === 0">{{ t("app.queued") }} <b>{{ steer }}</b></span>

      <div class="r" id="chrome-group">
        <button
          v-if="gathered"
          class="thbtn r-more"
          data-action="chrome.more"
          aria-haspopup="true"
          aria-controls="chrome-fold"
          :aria-expanded="more"
          :aria-label="t('common.more')"
          @click="more = !more"
        >
          <StudioIcon name="more" />
        </button>
        <div class="r-fold" id="chrome-fold" :data-open="more ? '' : undefined">
          <TooltipButton
            v-if="port"
            class="thbtn find-action"
            data-action="transcript.find"
            data-value="open"
            :aria-label="t('common.find_in_this_conversation')"
            :aria-keyshortcuts="ariaChord('f')"
            :title="t('common.find_in_this_conversation')"
            :shortcut="chord('F')"
            @click="onFind()"
          >
            <StudioIcon name="search" />
          </TooltipButton>
          <PhonePop v-if="hasHub && onError && hub" :hub="hub" :on-error="onError" />
          <TooltipButton
            class="thbtn theme-toggle"
            data-action="appearance.theme"
            :aria-label="theme === 'dark' ? t('app.switch_to_light_theme') : t('app.switch_to_dark_theme')"
            :title="theme === 'dark' ? t('app.switch_to_light_theme') : t('app.switch_to_dark_theme')"
            @click="onTheme()"
          >
            <StudioIcon :name="theme === 'dark' ? 'sun' : 'moon'" />
          </TooltipButton>
          <ViewSwitch v-if="port" :view="view" :rows="rows" :surfaces="surfaces" :on-view="onView" />
        </div>
        <WindowControls />
      </div>
    </div>
  </div>
</template>
