<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from "vue";
import { t } from "../../../i18n";
import { bytes } from "../../../i18n/format";
import type { AgentPort } from "../../../port/port";
import type { StoragePlan, StorageRoot } from "../../../port/storage";

// What the kernel keeps on disk, where each root lives and which of them may be
// moved. Rows are whatever it declares: a root added there shows up here without
// this panel being told about it.

// A root the kernel declares but this list has no name for still renders, under
// its own id — an unnamed row beats hiding storage someone is looking for.
const NAMED: Record<string, [string, string]> = {
  state: ["data.sessions_and_archives", "data.transcripts_compaction_archives_usage_stats"],
  cache: ["data.indexes_and_caches", "data.search_indexes_and_derived_data"],
  worktrees: ["data.isolated_worktrees", "data.the_separate_checkouts_delivery_works"],
  home: ["data.configuration_and_credentials", "data.settings_and_api_keys_these"],
  locks: ["data.process_locks", "data.how_instances_exclude_each_other"],
};

const PHASES: Record<string, string> = {
  adopting: "data.pointing_at_the_new_location",
  copying: "data.copying",
  verifying: "data.verifying",
  committed: "data.committed_clearing_the_old_location",
  done: "common.done_2",
};

const LAYOUT_LIMIT_MS = 10_000;
// Longer than the kernel's own per-root budget, so a root it gave up on arrives
// as a partial count and this only fires when the kernel says nothing at all.
const MEASURE_LIMIT_MS = 20_000;
const PLAN_DELAY_MS = 400;
const POLL_MS = 500;

function within<T>(work: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timeout")), ms);
    work.then(
      (value) => { clearTimeout(timer); resolve(value); },
      (e: unknown) => { clearTimeout(timer); reject(e instanceof Error ? e : new Error(String(e))); },
    );
  });
}

const props = defineProps<{ port: AgentPort }>();

const roots = ref<StorageRoot[]>([]);
const ready = ref(false);
const editable = ref(false);
const move = ref<{ root: string; to: string; phase: string; bytes: number; total: number; done: boolean; detail?: string; err?: string } | null>(null);
const leftBehind = ref<{ dir: string; names: string[] } | null>(null);
const stalled = ref<ReadonlySet<string>>(new Set<string>());
const error = ref("");
const open = ref("");
const target = ref("");
const plan = ref<StoragePlan | null>(null);
const busy = ref(false);

let generation = 0;
let disposed = false;
let planSeq = 0;
let planTimer: ReturnType<typeof setTimeout> | undefined;
let pollTimer: ReturnType<typeof setInterval> | undefined;

// Rows appear as soon as the kernel has laid them out; each root's size then
// arrives on its own, so one slow directory cannot hold the others back.
function read(): void {
  const mine = ++generation;
  error.value = "";
  stalled.value = new Set<string>();
  within(props.port.storage({ layout: true }), LAYOUT_LIMIT_MS)
    .then((layout) => {
      if (disposed || mine !== generation) return;
      roots.value = layout.roots;
      editable.value = layout.editable;
      move.value = layout.move ?? null;
      leftBehind.value = layout.leftBehind ?? null;
      ready.value = true;
      for (const root of layout.roots) {
        if (!root.pending) continue;
        within(props.port.storage({ root: root.id }), MEASURE_LIMIT_MS)
          .then((answer) => {
            if (disposed || mine !== generation) return;
            const measured = answer.roots[0];
            if (!measured) return;
            roots.value = roots.value.map((r) => (r.id === measured.id ? measured : r));
          })
          .catch(() => {
            if (disposed || mine !== generation) return;
            stalled.value = new Set<string>([...stalled.value, root.id]);
          });
      }
    })
    .catch(() => {
      if (!disposed && mine === generation) error.value = t("data.cannot_read_what_is_on");
    });
}

watch(() => props.port, () => { ready.value = false; read(); }, { immediate: true });

// A move outlives the request that started it, so the panel follows it the way
// it follows a running turn: by asking again until it says it is done. The poll
// carries the move and leaves the sizes already measured in place.
const running = computed(() => !!move.value && !move.value.done);

watch(running, (now) => {
  if (pollTimer !== undefined) { clearInterval(pollTimer); pollTimer = undefined; }
  if (!now) { read(); return; }
  pollTimer = setInterval(() => {
    within(props.port.storage({ layout: true }), LAYOUT_LIMIT_MS)
      .then((layout) => {
        if (disposed) return;
        move.value = layout.move ?? null;
        leftBehind.value = layout.leftBehind ?? null;
      })
      .catch(() => { if (!disposed) error.value = t("data.cannot_read_what_is_on"); });
  }, POLL_MS);
});

onScopeDispose(() => {
  disposed = true;
  generation++;
  clearTimeout(planTimer);
  clearInterval(pollTimer);
});

function begin(root: string): void {
  open.value = root;
  target.value = "";
  plan.value = null;
  clearTimeout(planTimer);
}

function cancel(): void {
  open.value = "";
  target.value = "";
  plan.value = null;
  clearTimeout(planTimer);
  planSeq++;
}

// The preflight runs behind a debounce: every keystroke in the destination field
// would otherwise be one kernel round trip, and the last answer is the only one
// worth drawing.
function retarget(dir: string): void {
  target.value = dir;
  clearTimeout(planTimer);
  const seq = ++planSeq;
  plan.value = null;
  if (!dir.trim()) return;
  planTimer = setTimeout(() => {
    props.port.planStorageMove(open.value, dir.trim())
      .then((answer) => { if (!disposed && seq === planSeq) plan.value = answer; })
      .catch(() => { if (!disposed && seq === planSeq) plan.value = null; });
  }, PLAN_DELAY_MS);
}

function start(): void {
  if (!open.value || !target.value.trim()) return;
  const root = open.value;
  const dir = target.value.trim();
  busy.value = true;
  props.port.moveStorage(root, dir)
    .then((accepted) => {
      if (disposed) return;
      if (!accepted.ok) { plan.value = accepted; return; }
      cancel();
      read();
    })
    .catch(() => { if (!disposed) error.value = t("data.the_move_could_not_be"); })
    .finally(() => { busy.value = false; });
}

const shown = computed(() => roots.value.filter((r) => !r.missing || r.relocatable));
const largest = computed(() => Math.max(1, ...roots.value.filter((r) => !r.pending && !stalled.value.has(r.id)).map((r) => r.bytes)));
const width = (root: StorageRoot) => `${Math.max(1, Math.round((root.bytes / largest.value) * 100))}%`;

// Which disks these add up on. Roots sharing a volume are one line, because
// moving one of them off a full drive only helps if the others go too.
const drives = computed(() => {
  const seen = new Map<string, StorageRoot>();
  for (const root of roots.value) {
    if (root.volume && root.volumeTotal && !seen.has(root.volume)) seen.set(root.volume, root);
  }
  return [...seen.values()];
});

const says = (root: StorageRoot) =>
  stalled.value.has(root.id)
    ? t("data.measuring_timed_out")
    : root.pending
      ? t("data.measuring")
      : (root.truncated ? "≥ " : "") + bytes(root.bytes);

const held = (root: StorageRoot) =>
  root.pinnedBy ? t("data.set_by_env", { env: root.pinnedBy }) : !root.relocatable ? t("data.cannot_be_moved") : "";
</script>

<template>
  <div class="storage">
    <div v-if="error" class="empty">{{ error }}</div>
    <div v-else-if="!ready" class="empty">{{ t("data.measuring") }}</div>
    <template v-else>
      <section class="grp">
        <h3 class="lbl">{{ t("data.space_used") }}</h3>
        <div v-for="root in shown" :key="root.id" class="vol" :data-state="stalled.has(root.id) ? 'stalled' : root.pending ? 'measuring' : root.truncated ? 'partial' : 'done'">
          <div class="row">
            <span class="nm">{{ t((NAMED[root.id] ?? [root.id, ""])[0]) }}</span>
            <span class="sz">
              {{ says(root) }}
              <template v-if="!root.pending && !stalled.has(root.id) && root.bytes > 0 && root.files > 0">
                · {{ t("common.n_files", { n: root.files }) }}
              </template>
            </span>
          </div>
          <p v-if="root.truncated && !root.pending && !stalled.has(root.id)" class="note">{{ t("data.this_folder_is_too_large") }}</p>
          <div v-if="!root.pending && !stalled.has(root.id)" class="meter"><i :style="{ width: width(root) }" /></div>
        </div>
        <div v-if="drives.length > 0" class="drives">
          <p v-for="root in drives" :key="root.volume">
            {{ t("data.drive_free_free_of_total", { drive: root.volume ?? "", free: bytes(root.volumeFree ?? 0), total: bytes(root.volumeTotal ?? 0) }) }}
          </p>
        </div>
      </section>

      <section v-if="leftBehind" class="grp">
        <h3 class="lbl">{{ t("data.data_remains_in_the_previous") }}</h3>
        <p class="note">{{ t("data.these_are_still_in_dir", { dir: leftBehind.dir, names: leftBehind.names.join("、") }) }}</p>
      </section>

      <section class="grp">
        <h3 class="lbl">{{ t("data.locations") }}</h3>
        <p v-if="!editable" class="note">{{ t("data.this_server_does_not_open") }}</p>
        <div v-for="root in roots" :key="root.id" class="item">
          <div class="l">
            <b>{{ t((NAMED[root.id] ?? [root.id, ""])[0]) }}</b>
            <span v-if="(NAMED[root.id] ?? ['', ''])[1]" class="hint">{{ t((NAMED[root.id] ?? ['', ''])[1]) }}</span>
            <span class="p">{{ root.dir }}</span>
            <span v-if="held(root)" class="held">{{ held(root) }}</span>
            <span v-if="root.err" class="held">{{ root.err }}</span>
          </div>
          <button v-if="root.relocatable && !root.pinnedBy && editable && open !== root.id" type="button" class="btn" data-action="storage.move-open" :data-target="root.id" :disabled="busy || running" @click="begin(root.id)">
            {{ t("data.move") }}
          </button>
          <div v-if="open === root.id" class="mover">
            <input
              data-action="storage.move"
              data-value="path"
              :value="target"
              :spellcheck="false"
              :placeholder="t('data.full_path_to_the_destination')"
              :aria-label="t('data.full_path_to_the_destination')"
              @input="retarget(($event.target as HTMLInputElement).value)"
            />
            <p v-if="plan && !plan.ok" class="ready" role="alert">{{ (plan.refusals ?? []).map((r) => r.detail).join(" ") }}</p>
            <p v-else-if="plan && plan.adopt" class="ready">
              {{ t("data.this_folder_already_holds_that", { size: bytes(plan.bytes), n: plan.files }) }}
              <template v-if="(plan.stays ?? 0) > 0"> {{ t("data.the_current_location_still_holds", { size: bytes(plan.stays ?? 0) }) }}</template>
            </p>
            <p v-else-if="plan" class="ready">{{ t("data.moves_size_in_n_files", { size: bytes(plan.bytes), n: plan.files, free: bytes(plan.free) }) }}</p>
            <div class="acts">
              <button type="button" class="btn" data-action="storage.move-cancel" @click="cancel">{{ t("common.cancel") }}</button>
              <button type="button" class="btn pri" data-action="storage.move-start" data-primary :disabled="!plan?.ok || busy" @click="start">
                {{ t(plan?.adopt ? "data.point_it_here" : "data.start_the_migration") }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <section v-if="move" class="grp moving">
        <h3 class="lbl">{{ t("data.migrate") }}</h3>
        <p v-if="move.err" class="failed">{{ move.err }}</p>
        <template v-else>
          <div class="row">
            <span class="nm">{{ t(PHASES[move.phase] ?? move.phase) }}</span>
            <span class="sz">{{ bytes(move.bytes) }} / {{ bytes(move.total) }}</span>
          </div>
          <div class="meter"><i :style="{ width: `${move.total > 0 ? Math.min(100, Math.round((move.bytes / move.total) * 100)) : 0}%` }" /></div>
          <p v-if="move.done" class="ready">{{ t("data.moved_it_takes_effect_after") }}</p>
          <p v-if="move.detail" class="ready">{{ move.detail }}</p>
        </template>
      </section>
    </template>
  </div>
</template>
