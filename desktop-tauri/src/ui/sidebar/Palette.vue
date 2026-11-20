<script lang="ts">
import type { StudioIconName } from "../StudioIcon.vue";
import type { TreeWorkspace } from "../../port/hub";

export interface Command {
  id: string;
  label: string;
  icon: StudioIconName;
  /** The terms someone would reach for instead of the label. */
  keywords: string;
  /** The shortcut that does the same thing, printed where the note would be. */
  keys?: string;
  run: () => void;
}

/** One made-to-fit overlay that asks conversations and actions at once. The
 *  column owns no list of its own here: what it can do is what it already
 *  offers, named once, so the two cannot drift into two spellings of it. */
export interface PaletteProps {
  open: boolean;
  commands: Command[];
  tree: TreeWorkspace[];
  onOpenSession: (path: string) => void;
  onClose: () => void;
}

export interface Hit {
  key: string;
  label: string;
  note: string;
  icon: StudioIconName;
  group: "actions" | "sessions";
  run: () => void;
}

const LIMIT = 6;

/** Both kinds of answer, in the order the groups are drawn. Pure, so the
 *  ranking can be checked without a renderer. */
export function hitsFor(commands: readonly Command[], tree: readonly TreeWorkspace[], query: string): Hit[] {
  const q = query.trim().toLowerCase();
  const actions: Hit[] = commands
    .filter((command) => !q || `${command.label} ${command.keywords}`.toLowerCase().includes(q))
    .slice(0, LIMIT)
    .map((command) => ({
      key: `c:${command.id}`,
      label: command.label,
      note: command.keys ?? "",
      icon: command.icon,
      group: "actions" as const,
      run: command.run,
    }));
  // A search word asks about titles and project names only: the transcript is
  // the content search's question, and it answers from the kernel.
  const sessions: Hit[] = [];
  for (const workspace of tree) {
    for (const session of workspace.sessions) {
      if (sessions.length >= LIMIT) break;
      const title = session.title || session.name;
      if (q && !`${title} ${workspace.name}`.toLowerCase().includes(q)) continue;
      sessions.push({
        key: `s:${session.path}`,
        label: title,
        note: workspace.name,
        icon: "clock",
        group: "sessions" as const,
        run: () => undefined,
      });
    }
  }
  return [...actions, ...sessions];
}
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import { useModalFocus } from "../useModalFocus";

const props = defineProps<PaletteProps>();

const sheet = ref<HTMLElement | null>(null);
const box = ref<HTMLInputElement | null>(null);
const query = ref("");
const at = ref(0);
useModalFocus(sheet, () => props.open);

const hits = computed(() => hitsFor(props.commands, props.tree, query.value));
// A cursor past the end of a shorter list selects nothing and Enter does
// nothing, which reads as the palette having stopped working.
const cursor = computed(() => (hits.value.length === 0 ? -1 : Math.min(at.value, hits.value.length - 1)));

const groups = computed(() => [
  { key: "actions", label: t("app.quick_actions"), hits: hits.value.filter((hit) => hit.group === "actions") },
  { key: "sessions", label: t("common.session"), hits: hits.value.filter((hit) => hit.group === "sessions") },
].filter((group) => group.hits.length > 0));

function pick(hit: Hit | undefined) {
  if (!hit) return;
  props.onClose();
  // A conversation row names a path rather than carrying its own closure: the
  // list is built from data, and the opener is the window's.
  if (hit.group === "sessions") props.onOpenSession(hit.key.slice(2));
  else hit.run();
}

function move(step: number) {
  if (hits.value.length === 0) return;
  const from = cursor.value < 0 ? 0 : cursor.value;
  at.value = (from + step + hits.value.length) % hits.value.length;
}

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    event.preventDefault();
    props.onClose();
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    pick(hits.value[cursor.value]);
    return;
  }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  move(event.key === "ArrowDown" ? 1 : -1);
}

// Every opening starts empty: a palette that reopens on the last word answers a
// question nobody asked this time.
watch(() => props.open, (open) => {
  if (!open) return;
  query.value = "";
  at.value = 0;
  void nextTick(() => box.value?.focus());
});

onMounted(() => {
  if (props.open) void nextTick(() => box.value?.focus());
});
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="palette-veil" role="presentation" @mousedown.self="onClose">
      <div ref="sheet" class="palette" role="dialog" aria-modal="true" :aria-label="t('app.search_and_commands')" @keydown="onKey">
        <label class="palette-input">
          <StudioIcon name="search" />
          <input
            ref="box"
            type="text"
            spellcheck="false"
            autocomplete="off"
            data-action-keydown="palette.search"
            :value="query"
            :aria-label="t('app.search_conversations_or_actions')"
            :placeholder="t('app.search_conversations_or_actions')"
            @input="query = ($event.target as HTMLInputElement).value; at = 0"
          />
          <kbd class="palette-esc" aria-hidden="true">Esc</kbd>
        </label>
        <div class="palette-results">
          <div v-for="group in groups" :key="group.key">
            <div class="palette-label">{{ group.label }}</div>
            <button
              v-for="hit in group.hits"
              :key="hit.key"
              type="button"
              class="palette-hit"
              data-action="palette.pick"
              :data-target="hit.key"
              :aria-selected="hits[cursor]?.key === hit.key"
              @mousemove="at = hits.findIndex((h) => h.key === hit.key)"
              @click="pick(hit)"
            >
              <StudioIcon :name="hit.icon" />
              <span>{{ hit.label }}</span>
              <small v-if="hit.note">{{ hit.note }}</small>
            </button>
          </div>
          <div v-if="hits.length === 0" class="palette-empty">{{ t("app.nothing_matches") }}</div>
        </div>
        <div class="palette-help"><span>{{ t("app.palette_keys") }}</span><span>{{ t("app.palette_scopes") }}</span></div>
      </div>
    </div>
  </Teleport>
</template>
