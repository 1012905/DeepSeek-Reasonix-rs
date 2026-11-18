<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { MemoryCatalog, MemoryEdit, MemoryEntry, MemoryProactive } from "../../../port/memory";
import StudioIcon from "../../StudioIcon.vue";

// The facts the agent wrote down about this project, and whether the last turn
// actually pulled one back. Everything here is a saved fact: the panel never
// decides what is true, it only lists, edits and archives what the store holds.
const PAGE = 30;

const TYPES: [string, string][] = [
  ["user", "remote.user"],
  ["feedback", "feedback.feedback"],
  ["project", "common.project"],
  ["reference", "memory.reference"],
  ["constraint", "memory.constraints"],
];
const TYPE_NAME: Record<string, string> = Object.fromEntries(TYPES);
const IMPORTANCE = ["", "memory.low", "memory.medium", "memory.high"];
const SCOPE: Record<string, string> = { project: "session.this_project", global: "memory.all_projects" };
const PROACTIVE: [MemoryProactive, string][] = [
  ["off", "memory.proactive_off"],
  ["ask", "memory.proactive_ask"],
  ["auto", "memory.proactive_auto"],
];

// A fact with no type reads as a project fact: that is what one written before
// the field existed was, and the list has no "unknown" bucket.
const typeOf = (m: MemoryEntry): string => (m.type && TYPE_NAME[m.type] ? m.type : "project");

type Filter = { kind: "all" | "new" | "constraint" | "faded" } | { kind: "type" | "galaxy"; value: string };

const sameFilter = (a: Filter, b: Filter) =>
  a.kind === b.kind && ("value" in a ? a.value : "") === ("value" in b ? b.value : "");

const matches = (m: MemoryEntry, f: Filter): boolean => {
  switch (f.kind) {
    case "all": return true;
    case "new": return !!m.new;
    case "constraint": return typeOf(m) === "constraint";
    case "faded": return m.state === "faded" && !m.noDecay;
    case "type": return typeOf(m) === f.value;
    case "galaxy": return (m.galaxy ?? "") === f.value;
  }
};

const searched = (m: MemoryEntry, q: string): boolean => {
  if (!q) return true;
  const hay = [m.title, m.name, m.description, m.body, m.galaxy].join("\n").toLowerCase();
  return q.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
};

const recency = (m: MemoryEntry) => m.lastUsedAt || m.updatedAt || m.createdAt || "";
const clip = (s: string) => (s.trim().length > 24 ? s.trim().slice(0, 24) + "…" : s.trim());
const clipBody = (s: string | undefined) => {
  const body = (s ?? "").trim();
  if (!body) return t("memory.no_body");
  return body.length > 200 ? body.slice(0, 200) + "…" : body;
};

const props = defineProps<{ port: AgentPort; onError: (e: unknown) => void }>();

const items = ref<MemoryEntry[] | null>(null);
const unreadable = ref<{ name: string; path: string; error: string }[]>([]);
const recall = ref("");
const unread = ref("");
const filter = ref<Filter>({ kind: "all" });
const search = ref("");
const shown = ref(PAGE);
const open = ref("");
const busy = ref<ReadonlySet<string>>(new Set<string>());
const error = ref("");
const edit = ref<MemoryEdit | null>(null);
const past = ref<Record<string, MemoryEntry[]>>({});
const openHistory = ref("");
const proactive = ref<MemoryProactive | null>(null);
const proactiveError = ref("");
let generation = 0;

function reload(): void {
  const mine = ++generation;
  props.port.memories()
    .then((c: MemoryCatalog) => {
      if (mine !== generation) return;
      items.value = c.memories;
      unreadable.value = c.unreadable ?? [];
      recall.value = c.recallQuery;
      unread.value = "";
    })
    .catch((e) => {
      if (mine !== generation) return;
      items.value = null;
      unreadable.value = [];
      unread.value = reason(e);
    });
}

watch(() => props.port, (port) => {
  reload();
  let live = true;
  port.memoryProactive().then((m) => { if (live) proactive.value = m; }).catch((e) => { if (live) proactiveError.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

const galaxies = computed(() => {
  const count = new Map<string, number>();
  for (const m of items.value ?? []) if (m.galaxy) count.set(m.galaxy, (count.get(m.galaxy) ?? 0) + 1);
  return [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
});

const list = computed(() => (items.value ?? [])
  .filter((m) => matches(m, filter.value) && searched(m, search.value.trim()))
  .sort((a, b) => recency(b).localeCompare(recency(a)) || (a.title || a.name).localeCompare(b.title || b.name)));

const page = computed(() => list.value.slice(0, shown.value));
const selected = computed(() => list.value.find((m) => m.name === open.value) ?? (filter.value.kind === "new" ? undefined : page.value[0]));
// The detail pane is drawn only when something is selected; the alias keeps the
// template's nested branches from having to re-narrow the same value.
const current = computed(() => selected.value as MemoryEntry);
const usedCount = computed(() => (items.value ?? []).filter((m) => m.usedLastTurn).length);
const count = (f: Filter) => (items.value ?? []).filter((m) => matches(m, f)).length;

const pick = (f: Filter) => { filter.value = f; shown.value = PAGE; };

async function run(name: string, action: () => Promise<void>): Promise<void> {
  busy.value = new Set([...busy.value, name]);
  error.value = "";
  try {
    await action();
    delete past.value[name];
    if (openHistory.value === name) openHistory.value = "";
    reload();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = new Set([...busy.value].filter((x) => x !== name));
  }
}

const save = () => {
  const held = edit.value;
  if (!held) return;
  void run(held.name, async () => { await props.port.saveMemory(held); edit.value = null; });
};

async function history(name: string): Promise<void> {
  if (openHistory.value === name) { openHistory.value = ""; return; }
  openHistory.value = name;
  error.value = "";
  if (past.value[name]) return;
  try {
    past.value = { ...past.value, [name]: await props.port.memoryRevisions(name) };
  } catch (e) {
    error.value = reason(e);
    openHistory.value = "";
  }
}

const beginEdit = (m: MemoryEntry) => {
  edit.value = {
    name: m.name, title: m.title ?? "", description: m.description ?? "", body: m.body ?? "",
    activation: m.activation, type: typeOf(m), galaxy: m.galaxy ?? "", importance: m.importance ?? 2,
  };
};

function setProactive(next: MemoryProactive): void {
  const was = proactive.value;
  proactive.value = next;
  proactiveError.value = "";
  props.port.setMemoryProactive(next).catch((e) => { proactive.value = was; proactiveError.value = reason(e); });
}

const railItem = (f: Filter, label: string, icon: string) => ({ f, label, icon, at: count(f), on: sameFilter(filter.value, f) });
const rail = computed(() => [
  railItem({ kind: "all" }, t("common.all"), "list"),
  railItem({ kind: "new" }, t("memory.new"), "spark"),
  railItem({ kind: "constraint" }, t("memory.constraints"), "lock"),
  ...TYPES.filter(([id]) => id !== "constraint").map(([id, name]) => railItem({ kind: "type", value: id }, t(name), "bookmark")),
  ...galaxies.value.map(([g]) => railItem({ kind: "galaxy", value: g }, g, "galaxy")),
  railItem({ kind: "faded" }, t("memory.faded"), "fading"),
]);

const patch = (p: Partial<MemoryEdit>) => { if (edit.value) edit.value = { ...edit.value, ...p }; };
</script>

<template>
  <div class="mem" :data-view="filter.kind">
    <!-- One line at the top: what the model does when it sees something worth
         keeping for good. Absent where the kernel has no such setting. -->
    <div v-if="proactive !== null || proactiveError" class="lrow">
      <span class="tx">
        <span class="lb">{{ t("memory.proactive_title") }}</span>
        <span class="ds">{{ t("memory.proactive_hint") }}</span>
        <span v-if="proactiveError" class="ds" role="alert">{{ proactiveError }}</span>
      </span>
      <div class="seg" data-text role="radiogroup" :aria-label="t('memory.proactive_title')">
        <button v-for="[id, label] in PROACTIVE" :key="id" type="button" role="radio" :aria-checked="proactive === id" :data-on="proactive === id ? '' : undefined" data-action="memory.proactive" :data-target="id" @click="setProactive(id)">
          {{ t(label) }}
        </button>
      </div>
    </div>

    <div v-if="unreadable.length > 0" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("memory.these_memories_could_not_be") }}</span>
      <span class="why">{{ unreadable.map((u) => `${u.name}: ${u.error}`).join("；") }}</span>
    </div>

    <div v-if="unread" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("memory.memory_could_not_be_read") }}</span>
      <span class="why">{{ unread }}<button type="button" class="lnk" data-action="memory.reload" @click="reload">{{ t("common.retry") }}</button></span>
    </div>
    <div v-else-if="!items" class="empty">{{ t("memory.loading_memory") }}</div>
    <div v-else-if="items.length === 0" class="empty">{{ t("memory.no_entries_yet") }}</div>

    <template v-else>
      <p v-if="recall" class="recall">
        {{ t("memory.last_turn_searched_memory_from", { q: clip(recall) }) }}
        {{ usedCount > 0 ? t("memory.n_of_them_used", { n: usedCount }) : t("memory.none_of_them_used") }}
      </p>
      <div class="memgrid">
        <nav class="memrail" :aria-label="t('memory.memory_categories')">
          <button
            v-for="item in rail"
            :key="item.label"
            type="button"
            class="memnav"
            data-action="memory.filter"
            :data-target="'value' in item.f ? item.f.value : item.f.kind"
            :aria-pressed="item.on"
            @click="pick(item.f)"
          >
            <StudioIcon :name="item.icon as 'list'" />
            <span class="lb">{{ item.label }}</span>
            <span class="c">{{ item.at }}</span>
          </button>
        </nav>

        <div class="memlist">
          <label class="memsearch">
            <StudioIcon name="search" :size="14" />
            <input type="search" data-action="memory.search" :placeholder="t('memory.search_memory')" :aria-label="t('memory.search_memory')" :value="search" @input="search = ($event.target as HTMLInputElement).value; shown = PAGE" />
          </label>
          <div v-if="page.length === 0" class="empty">{{ t(filter.kind === "new" ? "memory.nothing_is_waiting_for_you" : "memory.no_memories_match") }}</div>
          <div v-for="m in page" :key="m.name" class="memrow" :data-target="m.name" :data-used="m.usedLastTurn ? '' : undefined" :aria-current="selected?.name === m.name ? 'true' : undefined" :data-dim="m.state === 'faded' && !m.noDecay ? '' : undefined">
            <button type="button" class="nm" data-action="memory.open" :data-target="m.name" @click="open = m.name">
              <span class="txt">
                <span class="line">
                  <span class="ti">{{ m.title || m.name }}</span>
                  <span v-if="m.new" class="badge">{{ t("common.new") }}</span>
                  <span v-if="!m.noDecay && m.state === 'fuzzy'" class="fade">{{ t("memory.fading") }}</span>
                  <span v-else-if="!m.noDecay && m.state === 'faded'" class="fade">{{ t("memory.faded") }}</span>
                </span>
                <span class="gx">
                  {{ t(TYPE_NAME[typeOf(m)]) }} · {{ m.galaxy || t("memory.no_galaxy") }}
                  <template v-if="filter.kind === 'new' && m.createdAt">{{ t("memory.from_the_d_conversation", { d: m.createdAt.slice(5) }) }}</template>
                </span>
              </span>
              <i v-if="m.usedLastTurn" class="dot" :title="t('memory.used_last_turn')" />
            </button>
            <span v-if="filter.kind === 'new'" class="quick">
              <button type="button" class="memchip" data-action="memory.confirm" :data-target="m.name" :disabled="busy.has(m.name)" @click="run(m.name, () => port.confirmMemory(m.name))">
                <StudioIcon name="check" :size="13" />{{ t("memory.right") }}
              </button>
              <button type="button" class="memchip" data-action="memory.forget" :data-target="m.name" :disabled="busy.has(m.name)" @click="run(m.name, () => port.forgetMemory(m.name))">
                <StudioIcon name="close" :size="13" />{{ t("memory.forget") }}
              </button>
            </span>
          </div>
          <button v-if="list.length > shown" type="button" class="lnk more" data-action="memory.more" @click="shown += PAGE">
            {{ t("memory.show_n_more_total_in", { n: Math.min(PAGE, list.length - shown), total: list.length }) }}
          </button>
        </div>

        <div v-if="selected" class="memdetail" :data-target="current.name">
          <div v-if="edit && edit.name === current.name" class="memedit">
            <label><span>{{ t("memory.titling") }}</span><input data-action="memory.draft" data-value="title" :value="edit.title" @input="patch({ title: ($event.target as HTMLInputElement).value })" /></label>
            <label><span>{{ t("memory.one_line_summary") }}</span><input data-action="memory.draft" data-value="description" :value="edit.description" @input="patch({ description: ($event.target as HTMLInputElement).value })" /></label>
            <label><span>{{ t("common.body_text") }}</span><textarea rows="8" data-action="memory.draft" data-value="body" :value="edit.body" @input="patch({ body: ($event.target as HTMLTextAreaElement).value })" /></label>
            <div class="pair">
              <label>
                <span>{{ t("common.type") }}</span>
                <select data-action="memory.draft" data-value="type" :value="edit.type" @change="patch({ type: ($event.target as HTMLSelectElement).value })">
                  <option v-for="[id, name] in TYPES" :key="id" :value="id">{{ t(name) }}</option>
                </select>
              </label>
              <label>
                <span>{{ t("memory.importance") }}</span>
                <select data-action="memory.draft" data-value="importance" :value="String(edit.importance ?? 2)" @change="patch({ importance: Number(($event.target as HTMLSelectElement).value) })">
                  <option v-for="n in [3, 2, 1]" :key="n" :value="n">{{ t(IMPORTANCE[n]) }}</option>
                </select>
              </label>
            </div>
            <label>
              <span>{{ t("memory.galaxy") }}</span>
              <input list="rx-mem-galaxies" data-action="memory.draft" data-value="galaxy" :value="edit.galaxy ?? ''" @input="patch({ galaxy: ($event.target as HTMLInputElement).value })" />
              <datalist id="rx-mem-galaxies"><option v-for="[g] in galaxies" :key="g" :value="g" /></datalist>
            </label>
            <label>
              <span>{{ t("memory.when_it_applies") }}</span>
              <select data-action="memory.draft" data-value="activation" :value="edit.activation" @change="patch({ activation: ($event.target as HTMLSelectElement).value })">
                <option value="relevant">{{ t("memory.recalled_when_relevant") }}</option>
                <option value="pinned">{{ t("memory.in_every_turn") }}</option>
              </select>
            </label>
            <div class="row">
              <button type="button" class="btn" data-action="memory.save" :data-target="current.name" :disabled="busy.has(current.name)" @click="save">
                {{ t(busy.has(current.name) ? "common.saving_2" : "common.save") }}
              </button>
              <button type="button" class="btn" data-action="memory.cancel" :data-target="current.name" @click="edit = null">{{ t("common.cancel") }}</button>
              <button type="button" class="btn" data-action="memory.forget" :data-target="current.name" :disabled="busy.has(current.name)" @click="run(current.name, () => port.forgetMemory(current.name))">
                {{ t("common.archived_2") }}
              </button>
              <span class="hint">{{ t("memory.saving_records_a_new_revision") }}</span>
            </div>
          </div>

          <template v-else>
            <div class="memhead">
              <div class="memhead-tx">
                <div class="ti">{{ current.title || current.name }}</div>
                <p v-if="current.description" class="ds">{{ current.description }}</p>
              </div>
              <button type="button" class="btn" data-action="memory.confirm" :data-target="current.name" :disabled="busy.has(current.name)" @click="run(current.name, () => port.confirmMemory(current.name))">
                {{ t("memory.thats_right") }}
              </button>
              <button type="button" class="btn" data-action="memory.edit" :data-target="current.name" @click="beginEdit(selected)">{{ t("common.edit") }}</button>
              <button type="button" class="btn" data-action="memory.forget" :data-target="current.name" :disabled="busy.has(current.name)" :aria-label="t('common.archived_2')" :title="t('common.archived_2')" @click="run(current.name, () => port.forgetMemory(current.name))">
                {{ t("common.archived_2") }}
              </button>
            </div>
            <p v-if="current.usedLastTurn && current.why" class="why-used">{{ t("memory.surfaced_last_turn_because_of", { why: current.why }) }}</p>
            <p v-if="current.expired" class="stale">{{ t("memory.expired") }}</p>
            <pre>{{ current.body?.trim() || t("memory.no_body") }}</pre>
            <div class="facts">
              <section :aria-label="t('memory.content')">
                <h4>{{ t("memory.content") }}</h4>
                <dl>
                  <div><dt>{{ t("common.type") }}</dt><dd>{{ t(TYPE_NAME[typeOf(selected)]) }}</dd></div>
                  <div><dt>{{ t("memory.galaxy") }}</dt><dd>{{ current.galaxy || t("memory.no_galaxy") }}</dd></div>
                  <div><dt>{{ t("memory.importance") }}</dt><dd>{{ t(IMPORTANCE[current.importance ?? 2] || "memory.medium") }}</dd></div>
                  <div><dt>{{ t("memory.scope") }}</dt><dd>{{ t(SCOPE[current.scope ?? ""] ?? current.scope ?? "") }}</dd></div>
                </dl>
              </section>
              <section :aria-label="t('memory.use')">
                <h4>{{ t("memory.use") }}</h4>
                <dl>
                  <div><dt>{{ t("memory.state") }}</dt><dd>{{ t(current.noDecay ? "memory.clear_and_it_never_fades" : current.state === "faded" ? "memory.faded_recalled_after_everything_else" : current.state === "fuzzy" ? "memory.a_little_fuzzy_using_it" : "memory.clear") }}</dd></div>
                  <div><dt>{{ t("session.recent") }}</dt><dd>{{ current.lastUsedAt || t("memory.not_recalled_yet") }}</dd></div>
                  <div><dt>{{ t("memory.used") }}</dt><dd>{{ t("memory.n_times", { n: current.uses ?? 0 }) }}</dd></div>
                  <div><dt>{{ t("memory.applies") }}</dt><dd>{{ t(current.activation === "pinned" ? "memory.in_every_turn" : "memory.recalled_when_relevant_2") }}</dd></div>
                </dl>
              </section>
            </div>
            <div class="row sub">
              <button v-if="(current.revision ?? 1) > 1" type="button" class="lnk" data-action="memory.history" :data-target="current.name" @click="history(current.name)">
                {{ t(openHistory === current.name ? "memory.hide_older_revisions" : "memory.revision_n_see_older", { n: current.revision ?? 1 }) }}
              </button>
              <span v-if="current.path" class="path">{{ current.path }}</span>
            </div>
            <div v-if="openHistory === current.name" class="memhist">
              <span v-if="!past[current.name]" class="ds">{{ t("memory.reading_older_revisions") }}</span>
              <span v-else-if="past[current.name].filter((x) => (x.revision ?? 1) !== (current.revision ?? 1)).length === 0" class="ds">{{ t("memory.only_the_current_revision_exists") }}</span>
              <template v-else>
                <div v-for="x in past[current.name].filter((y) => (y.revision ?? 1) !== (current.revision ?? 1))" :key="x.revision" class="histrow">
                  <span class="rev">{{ t("memory.revision_n", { n: x.revision ?? 1 }) }}</span>
                  <span class="at">{{ x.updatedAt || x.createdAt }}</span>
                  <span class="ds" :title="x.title">{{ x.title }}</span>
                  <button type="button" class="btn" data-action="memory.restore" :data-target="current.name" :disabled="busy.has(current.name)" @click="run(current.name, () => port.restoreMemory(current.name, x.revision ?? 1))">
                    {{ t("memory.restore_this_one") }}
                  </button>
                  <pre>{{ clipBody(x.body) }}</pre>
                </div>
                <span class="hint">{{ t("memory.restoring_records_a_new_revision") }}</span>
              </template>
            </div>
          </template>
        </div>
      </div>
    </template>

    <div v-if="error" class="why" role="alert">{{ error }}</div>
    <p class="note">{{ t("memory.to_write_one_yourself_use", { cmd: "/remember" }) }}</p>
  </div>
</template>
