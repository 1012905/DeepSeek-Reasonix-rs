<script lang="ts">
import type { SessionScope } from "../sessionScope";

/** The scope picker beside the project heading. It is a menu rather than a row
 *  of tabs because five choices do not fit a narrow column, and because the
 *  count beside each is a number someone reads before deciding. */
export interface SessionFilterProps {
  scope: SessionScope;
  counts: Record<SessionScope, number>;
  allowed: readonly SessionScope[];
  names: Record<SessionScope, string>;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { pinToViewport } from "../place";

const props = defineProps<SessionFilterProps>();
const emit = defineEmits<{ (e: "scope", value: SessionScope): void }>();

const trigger = ref<HTMLButtonElement | null>(null);
const menu = ref<HTMLElement | null>(null);
const open = ref(false);

const current = computed(() => t(props.names[props.scope] ?? "common.all"));

// The menu is drawn at the document root's own layer, so its corner is measured
// from the trigger rather than inherited from it: the column clips whatever is
// wider than itself.
function place() {
  const anchor = trigger.value?.getBoundingClientRect();
  if (!anchor || !menu.value) return;
  pinToViewport(menu.value, anchor.left, anchor.bottom + 4);
}

function close(refocus = true) {
  open.value = false;
  if (refocus) trigger.value?.focus();
}

function toggle() {
  if (open.value) {
    close();
    return;
  }
  open.value = true;
  void nextTick(() => {
    place();
    menu.value?.querySelector<HTMLElement>('button[role="menuitemradio"]')?.focus();
  });
}

function pick(value: SessionScope) {
  emit("scope", value);
  close();
}

function onMenuKey(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    event.preventDefault();
    close();
    return;
  }
  if (event.key === "Tab") {
    close(false);
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const rows = [...(menu.value?.querySelectorAll<HTMLButtonElement>('button[role="menuitemradio"]') ?? [])];
  if (rows.length === 0) return;
  const at = rows.indexOf(document.activeElement as HTMLButtonElement);
  const to = event.key === "Home" ? 0
    : event.key === "End" ? rows.length - 1
    : (at + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length;
  rows[to]?.focus();
}

// A press anywhere else closes the menu: a popover left hanging over the tree
// covers the very rows it was opened to filter.
watch(open, (isOpen, _old, onCleanup) => {
  if (!isOpen) return;
  const away = (event: MouseEvent) => {
    const at = event.target as Node;
    if (!menu.value?.contains(at) && !trigger.value?.contains(at)) close(false);
  };
  const follow = () => place();
  addEventListener("mousedown", away);
  addEventListener("resize", follow);
  addEventListener("scroll", follow, true);
  onCleanup(() => {
    removeEventListener("mousedown", away);
    removeEventListener("resize", follow);
    removeEventListener("scroll", follow, true);
  });
});
</script>

<template>
  <div class="session-filter">
    <button
      ref="trigger"
      type="button"
      class="session-filter-trigger"
      data-action="session.filter"
      :aria-label="t('session.session_scope')"
      :aria-expanded="open"
      aria-haspopup="menu"
      :title="t('session.session_scope')"
      @click="toggle"
    ><span>{{ current }}</span><StudioIcon name="down" /></button>
    <button
      v-if="scope !== 'all'"
      type="button"
      class="session-filter-clear"
      data-action="session.filter-clear"
      :aria-label="t('session.clear_filter')"
      :title="t('session.clear_filter')"
      @click="pick('all')"
    ><StudioIcon name="close" /></button>
    <Teleport to="body">
      <div v-if="open" ref="menu" class="session-pop session-filter-menu" role="menu" :aria-label="t('session.session_scope')" @keydown="onMenuKey" @click.stop>
        <div class="session-pop-group">
          <button
            v-for="value in allowed"
            :key="value"
            type="button"
            role="menuitemradio"
            :aria-checked="scope === value"
            :data-value="value"
            data-action="session.filter"
            @click="pick(value)"
          >
            <StudioIcon :name="scope === value ? 'check' : 'circle'" />
            <span>{{ t(names[value] ?? value) }}</span>
            <small>{{ counts[value] ?? 0 }}</small>
          </button>
        </div>
      </div>
    </Teleport>
  </div>
</template>
