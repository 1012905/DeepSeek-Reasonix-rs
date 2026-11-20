<script lang="ts">
// One remote project's own menu. Extracted from the row it hangs on so the row
// stays a row: the button here owns nothing but what opens under it.
export interface RemoteProjectMenuProps {
  name: string;
  root: string;
  host: string;
  /** This host's default folder, so the menu can say whether this one is it. */
  defaultRoot?: string;
  busy?: boolean;
  onRefresh?: () => void;
  /** Absent for a folder this window only knows from an open pane — there is
   *  nothing in the book to take out. */
  onRemove?: () => void;
  /** Promotes this folder to the host's default, which is the head of the list. */
  onMakeDefault?: () => void;
  /** A direction with no neighbour is absent rather than disabled. */
  onMove?: Partial<Record<-1 | 1, () => void>>;
}
</script>

<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import { t } from "../../i18n";
import { useDismiss } from "../dismiss";
import { pinToViewport } from "../place";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<RemoteProjectMenuProps>();

const open = ref(false);
const trigger = ref<HTMLButtonElement | null>(null);
const menu = ref<HTMLElement | null>(null);

function close(restore = true): void {
  open.value = false;
  if (restore) trigger.value?.focus();
}

useDismiss(open, trigger, () => close(), menu);

watch(open, async (shown) => {
  if (!shown) return;
  await nextTick();
  const el = menu.value;
  const at = trigger.value?.getBoundingClientRect();
  // The menu is fixed and rendered at the document root, so it is placed from
  // the trigger's on-screen box rather than by an ancestor's offset.
  if (el && at) pinToViewport(el, at.right + 8, at.top, 12);
  el?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
});

// Arrows walk the entries; Escape and Tab leave through the same door, and the
// caller's trigger gets the focus back so the next press does not restart at
// the top of the page.
function onKey(event: KeyboardEvent): void {
  if (event.key === "Escape" || event.key === "Tab") {
    event.stopPropagation();
    if (event.key === "Escape") event.preventDefault();
    close();
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  event.stopPropagation();
  const all = rows();
  if (!all.length) return;
  const at = all.indexOf(document.activeElement as HTMLButtonElement);
  const to = event.key === "Home" ? 0
    : event.key === "End" ? all.length - 1
      : (at + (event.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
  all[to]?.focus();
}

const rows = () => [...(menu.value?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)') ?? [])];

function take(run: (() => void) | undefined): void {
  if (!run) return;
  close();
  run();
}

onBeforeUnmount(() => { open.value = false; });
</script>

<template>
  <button
    ref="trigger"
    type="button"
    class="wsmore"
    data-action="workspace.menu"
    :data-target="host + ':' + root"
    :title="t('common.more_actions')"
    :aria-label="t('common.project_actions_name', { name: name + ' · ' + host })"
    aria-haspopup="menu"
    :aria-expanded="open"
    @click.stop="open = !open"
  >
    <StudioIcon name="more" />
  </button>

  <Teleport to="body">
    <div
      v-if="open"
      ref="menu"
      class="session-pop"
      role="menu"
      :aria-label="t('common.project_actions')"
      data-action-keydown="workspace.menu"
      @keydown="onKey"
      @click.stop
    >
      <div class="session-pop-head">
        <b>{{ name }}</b>
        <small class="session-pop-path">{{ host }} · {{ root }}</small>
        <small class="session-pop-path">
          {{ defaultRoot ? t("remote.default_connection_directory_path", { path: defaultRoot }) : t("remote.no_default_workspace") }}
        </small>
      </div>
      <div class="session-pop-group">
        <button v-if="onRefresh" role="menuitem" data-action="remote.read" :data-target="host" :disabled="busy" @click="take(onRefresh)">
          <StudioIcon name="refresh" /><span>{{ t("remote.refresh_the_sessions_on_name", { name: host }) }}</span>
        </button>
        <button
          v-if="onMakeDefault && root !== defaultRoot"
          role="menuitem"
          data-action="remote.make-default"
          :data-target="root"
          :disabled="busy"
          @click="take(onMakeDefault)"
        >
          <StudioIcon name="check" /><span>{{ t("remote.make_path_the_default", { path: root }) }}</span>
        </button>
        <button
          v-for="direction in ([-1, 1] as const)"
          v-show="onMove?.[direction]"
          :key="direction"
          role="menuitem"
          data-action="remote-host.move"
          :data-target="host"
          :data-value="direction"
          :disabled="busy"
          @click="take(onMove?.[direction])"
        >
          <StudioIcon name="arrow" :data-flip="direction === 1 ? '' : undefined" />
          <span>{{ t(direction === -1 ? "common.move_name_up" : "common.move_name_down", { name: host }) }}</span>
        </button>
        <button v-if="onRemove" role="menuitem" data-action="remote.remove-dir" :data-target="root" :disabled="busy" @click="take(onRemove)">
          <StudioIcon name="close" /><span>{{ t("common.remove_from_the_list") }}</span><small>{{ t("common.keeps_files") }}</small>
        </button>
      </div>
    </div>
  </Teleport>
</template>
