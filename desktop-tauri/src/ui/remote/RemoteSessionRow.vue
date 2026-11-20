<script lang="ts">
import type { TreeSession } from "../../port/hub";

// One conversation on another machine, as its own row. The caller has already
// decided which rows exist and in what order; this decides how one looks and
// which of its actions are offered.
export interface RemoteSessionRowProps {
  host: string;
  /** host + ":" + session path: one identity for this row across both stores. */
  target: string;
  session: TreeSession;
  /** The pane already drives this conversation, when one does. */
  held?: string;
  /** The pane on screen. */
  active?: string;
  busy?: boolean;
  /** A mutation of this row is in flight. */
  pending?: string;
  /** A turn is running in the pane that holds it. */
  live?: boolean;
  onOpen: () => void;
  /** Focuses the pane that already holds it, which is not opening a second one. */
  onFocus?: (id: string) => void;
  onWrite?: (kind: "pin" | "archive" | "read") => void;
  onDelete?: () => void;
  /** Where the menu should be anchored, so it lands beside the row. */
  onMenu?: (x: number, y: number) => void;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { LeadDot } from "../UnreadMark.vue";

const props = defineProps<RemoteSessionRowProps>();

const title = computed(() => props.session.title || props.session.name);
const on = computed(() => !!props.held && props.held === props.active);
const off = computed(() => !!props.pending || !!props.busy);
// An archive and a delete both stop at a running turn: the kernel refuses them
// anyway, and a button that only looks pressable is worse than one that is not.
const stuck = computed(() => !!props.live);

function pick(): void {
  if (props.pending) return;
  if (props.held && props.onFocus) props.onFocus(props.held);
  else props.onOpen();
}

function write(kind: "pin" | "archive" | "read"): void {
  if (off.value) return;
  if (kind === "archive" && stuck.value) return;
  props.onWrite?.(kind);
}

function menuAt(x: number, y: number): void {
  if (off.value) return;
  props.onMenu?.(x, y);
}

/** A press on the row's own more-button opens where the button is; the caller
 *  places the menu, so the anchor is a point rather than a ref it cannot see. */
function menuFromButton(event: MouseEvent): void {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuAt(rect.right, rect.top);
}

function onKey(event: KeyboardEvent): void {
  if (event.target !== event.currentTarget) return;
  if (off.value) return;
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    event.stopPropagation();
    pick();
  } else if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
    event.preventDefault();
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menuAt(rect.right, rect.top);
  }
}
</script>

<template>
  <div
    class="sessrow sessrow-remote ds-row"
    role="treeitem"
    :aria-level="2"
    :aria-label="title"
    :aria-selected="on"
    tabindex="-1"
    :data-target="target"
    :data-on="on ? '' : undefined"
    :data-live="held ? '' : undefined"
    :data-unread="session.unread && !on ? '' : undefined"
    :data-busy="pending || busy ? '' : undefined"
    data-action-click="session.open"
    data-action-contextmenu="session.menu"
    @click="pick"
    @contextmenu.prevent.stop="menuAt($event.clientX, $event.clientY)"
    @keydown="onKey"
  >
    <LeadDot :unread="session.unread && !on" />
    <span class="sesstitle ds-title" :title="title">
      <span>{{ title }}</span>
      <em v-if="session.isolated" class="sessarch" :title="session.workspaceRoot">{{ t("common.isolated") }}</em>
      <em v-if="session.archived" class="sessarch">{{ t("common.archived") }}</em>
    </span>
    <span v-if="session.pinned" class="ds-pin" :aria-label="t('common.pinned')" :title="t('common.pinned')">
      <StudioIcon name="pin" />
    </span>
    <!-- Hover controls: the ones the local rail offers, because an action
         missing on a remote row is one the reader goes to the far machine for. -->
    <span v-if="onWrite || onMenu" class="sessquick">
      <button
        v-if="onWrite"
        type="button"
        data-action="session.pin"
        :data-target="target"
        :disabled="off"
        :title="session.pinned ? t('common.unpin') : t('common.pin_session')"
        :aria-label="session.pinned ? t('common.unpin') : t('common.pin_session')"
        @click.stop="write('pin')"
      >
        <StudioIcon name="pin" />
      </button>
      <button
        v-if="onMenu"
        type="button"
        class="ds-more"
        data-action="session.menu"
        :data-target="target"
        :disabled="off"
        :title="t('common.more_actions')"
        :aria-label="t('common.session_actions')"
        @click.stop="menuFromButton"
      >
        <StudioIcon name="more" />
      </button>
    </span>
  </div>
</template>
