<script lang="ts">
import type { TreeWorkspace } from "../../port/hub";
import type { RecentRow } from "./workspaceTree";

/** The sheet that lists recent conversations across every project. Rows are
 *  settled by the caller, so this component only draws and reports a pick. */
export interface RecentSessionsProps {
  rows: RecentRow[];
  /** The project a row belongs to, named under its title. */
  workspaceName: (workspace: TreeWorkspace) => string;
  onPick: (path: string) => void;
  onClose: () => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { useModalFocus } from "../useModalFocus";

const props = defineProps<RecentSessionsProps>();

const sheet = ref<HTMLElement | null>(null);
const list = ref<HTMLElement | null>(null);
useModalFocus(sheet);

// Today is the only boundary this list needs: a conversation from this morning
// and one from last month are the two groups a reader scans for.
const start = new Date().setHours(0, 0, 0, 0);
const groups = computed(() => {
  const at = (row: RecentRow) => row.session.lastActiveAt ?? 0;
  return [
    { key: "today", label: t("common.today"), rows: props.rows.filter((row) => at(row) >= start) },
    { key: "earlier", label: t("session.earlier"), rows: props.rows.filter((row) => at(row) < start) },
  ];
});

function rowsIn() {
  return [...(list.value?.querySelectorAll<HTMLElement>(".recent-session:not(:disabled)") ?? [])];
}

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    event.preventDefault();
    props.onClose();
    return;
  }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  const all = rowsIn();
  const at = all.indexOf(document.activeElement as HTMLElement);
  // Out of the sheet's own controls and into the list on one press, and back
  // up out of the first row the same way.
  if (event.key === "ArrowDown") (at < 0 ? all[0] : all[Math.min(at + 1, all.length - 1)])?.focus();
  else if (at === 0) sheet.value?.querySelector<HTMLElement>("button")?.focus();
  else if (at > 0) all[at - 1].focus();
}

onMounted(() => { void nextTick(() => sheet.value?.focus()); });
</script>

<template>
  <Teleport to="body">
    <div class="pickveil" role="dialog" aria-modal="true" :aria-label="t('session.recent_sessions')" @mousedown.self="onClose">
      <div ref="sheet" class="pickcard recent-sessions" tabindex="-1" @keydown="onKey">
        <h2>{{ t("session.recent_sessions") }}</h2>
        <p class="recent-description">{{ t("session.ordered_by_last_activity_including") }}</p>
        <div ref="list" class="picklist">
          <p v-if="rows.length === 0" class="pickempty">{{ t("common.nothing_matches") }}</p>
          <div v-else>
            <template v-for="group in groups" :key="group.key">
              <template v-if="group.rows.length > 0">
                <p class="recent-group">{{ group.label }}</p>
                <button
                  v-for="row in group.rows"
                  :key="row.session.path"
                  type="button"
                  class="pickrow recent-session"
                  data-action="session.open"
                  :data-target="row.session.path"
                  :title="`${row.session.workspaceRoot || row.workspace.root}\n${row.session.path}`"
                  @click="onPick(row.session.path)"
                >
                  <StudioIcon name="chat" />
                  <span class="recent-session-text">
                    <b>{{ row.session.title || row.session.name }}</b>
                    <small>{{ workspaceName(row.workspace) }} · {{ row.session.workspaceRoot || row.workspace.root }}</small>
                  </span>
                </button>
              </template>
            </template>
          </div>
        </div>
        <p class="recent-keys">{{ t("session.to_choose_enter_to_open") }}</p>
        <div class="pickact">
          <button data-action="layer.dismiss" @click="onClose">{{ t("common.close") }}</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
