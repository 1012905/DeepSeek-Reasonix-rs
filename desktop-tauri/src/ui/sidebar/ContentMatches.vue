<script lang="ts">
import type { HistoryHit, HistoryIndex, HistorySearch, HubPort, TreeWorkspace } from "../../port/hub";

/** Conversations whose words, not names, match what the column is searching
 *  for. The name filter answers instantly; this adds what was said once the
 *  reader has paused, which is the only way a decision recorded in a transcript
 *  is findable at all. */
export interface ContentMatchesProps {
  hub: HubPort;
  tree: TreeWorkspace[];
  needle: string;
  onOpen: (path: string) => void;
  onError: (error: unknown) => void;
}

export const MIN_CHARS = 2;
export const SETTLE_MS = 300;

export function titleOf(hit: Pick<HistoryHit, "path" | "title" | "id">, tree: readonly TreeWorkspace[]): string {
  const listed = tree.flatMap((workspace) => workspace.sessions).find((session) => session.path === hit.path);
  return listed?.title || hit.title || listed?.name || hit.id || hit.path;
}

/** One line when the index could not answer for every conversation it knows.
 *  Without it, a source it failed to read and a query that matched nothing
 *  reach this column as the same empty list. */
export function indexNote(index: HistoryIndex | undefined): { reason: "failed" | "building"; key: string; vars?: Record<string, number> } | null {
  if (!index?.partial) return null;
  if (index.reason === "failed") return { reason: "failed", key: "chat.some_history_could_not_be" };
  return index.total > 0
    ? { reason: "building", key: "chat.the_history_index_is_still_2", vars: { indexed: index.indexed, total: index.total } }
    : { reason: "building", key: "chat.the_history_index_is_still" };
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<ContentMatchesProps>();

type Reading = "reading" | "failed" | HistorySearch | null;
const answer = ref<Reading>(null);
let generation = 0;

const asks = computed(() => props.needle.trim().length >= MIN_CHARS);

watch([() => props.needle, () => props.hub], ([needle]) => {
  const current = ++generation;
  answer.value = null;
  if (needle.trim().length < MIN_CHARS) return;
  // A pause before the kernel is asked: a word arrives one letter at a time and
  // every one of them would otherwise be a search of the whole history.
  const timer = setTimeout(() => {
    answer.value = "reading";
    props.hub.searchHistory(needle.trim()).then(
      (result) => { if (current === generation) answer.value = result; },
      () => { if (current === generation) answer.value = "failed"; },
    );
  }, SETTLE_MS);
  return () => clearTimeout(timer);
}, { immediate: true });

const reading = computed(() => answer.value === "reading");
const failed = computed(() => answer.value === "failed");
const hits = computed<HistoryHit[]>(() => (answer.value && typeof answer.value === "object" ? answer.value.hits : []));
const note = computed(() => (answer.value && typeof answer.value === "object" ? indexNote(answer.value.index) : null));
const done = computed(() => !reading.value && !failed.value && answer.value !== null);
</script>

<template>
  <div v-if="asks" class="content-matches">
    <p v-if="reading" role="status" data-state="reading">{{ t("chat.searching_conversation_text") }}</p>
    <p v-else-if="failed" role="status" data-state="failed">{{ t("chat.conversation_text_cannot_be_searched") }}</p>
    <template v-else-if="done">
      <h3>{{ t("chat.matches_in_conversation_text") }}</h3>
      <p v-if="note" class="content-matches-note" role="status" data-state="incomplete" :data-reason="note.reason">
        {{ note.vars ? t(note.key, note.vars) : t(note.key) }}
      </p>
      <ul v-if="hits.length > 0">
        <li v-for="hit in hits" :key="hit.path">
          <button type="button" data-action="session.open-match" :data-target="hit.path" @click="onOpen(hit.path)">
            <b>{{ titleOf(hit, tree) }}</b>
            <span>{{ hit.snippet }}</span>
          </button>
        </li>
      </ul>
      <p v-else-if="!note" class="content-matches-note"><StudioIcon name="search" />{{ t("common.nothing_matches") }}</p>
    </template>
  </div>
</template>
